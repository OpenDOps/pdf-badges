use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use crate::registration_server::cups::{self, PrintRequest, Search};
use crate::registration_server::current::CurrentEvent;
use crate::registration_server::gate::Gate;
use crate::registration_server::ServeError;

/// Jobs waiting on the server thread. One worker drains the queue.
pub const PRINT_QUEUE_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PrintJob {
    pub id: String,
    pub pages: u32,
    /// Zone route. Absent leaves the worker to read `zone` from the visitor.
    #[serde(default)]
    pub zone: Option<String>,
    /// Explicit queue name. It wins when that queue is connected.
    #[serde(default)]
    pub name: Option<String>,
    /// Set by `POST /api/print`. Absent reads the flag from the visitor row.
    #[serde(default)]
    pub certificate: Option<bool>,
    /// Set when the print body names a category. Absent reads the visitor row.
    #[serde(default)]
    pub category: Option<i64>,
}

#[derive(Clone)]
pub struct PrintQueue {
    tx: mpsc::Sender<PrintJob>,
}

pub struct PrintWorker {
    rx: mpsc::Receiver<PrintJob>,
    gate: Arc<Gate>,
}

impl PrintWorker {
    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Option<PrintJob> {
        self.rx.try_recv().ok()
    }
}

#[derive(Debug)]
pub enum EnqueueError {
    EmptyId,
    NoPages,
    Full,
    Closed,
}

impl PrintQueue {
    pub fn pair(gate: Arc<Gate>) -> (Self, PrintWorker) {
        let (tx, rx) = mpsc::channel(PRINT_QUEUE_DEPTH);
        (Self { tx }, PrintWorker { rx, gate })
    }

    /// A job is sitting in the channel. A job the worker has already received is the gate's print flag.
    pub(crate) fn has_queued(&self) -> bool {
        self.tx.capacity() < self.tx.max_capacity()
    }

    pub fn enqueue(&self, job: PrintJob) -> Result<(), EnqueueError> {
        if job.id.is_empty() {
            return Err(EnqueueError::EmptyId);
        }
        if job.pages == 0 {
            return Err(EnqueueError::NoPages);
        }
        self.tx.try_send(job).map_err(|err| match err {
            mpsc::error::TrySendError::Full(_) => EnqueueError::Full,
            mpsc::error::TrySendError::Closed(_) => EnqueueError::Closed,
        })
    }
}

pub async fn run(
    mut worker: PrintWorker,
    current: Arc<Mutex<CurrentEvent>>,
    search: Search,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), ServeError> {
    loop {
        tokio::select! {
            biased;
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            job = worker.rx.recv() => {
                let Some(job) = job else {
                    return Ok(());
                };
                if let Err(err) = print_one(&search, &current, &job, &worker.gate).await {
                    log::debug!("print {}: {err}", job.id);
                }
            }
            _ = wait_for_flush(&search) => {
                if let Err(err) = search.flush_due(&worker.gate).await {
                    log::debug!("print window: {err}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    }
}

async fn wait_for_flush(search: &Search) {
    match search.flush_wait() {
        Some(pause) => tokio::time::sleep(pause).await,
        None => std::future::pending::<()>().await,
    }
}

async fn print_one(
    search: &Search,
    current: &Mutex<CurrentEvent>,
    job: &PrintJob,
    gate: &Arc<Gate>,
) -> Result<cups::Printed, cups::Error> {
    let request = load_request(search, current, job)?;
    search.print(&request, gate).await
}

fn load_request(
    search: &Search,
    current: &Mutex<CurrentEvent>,
    job: &PrintJob,
) -> Result<PrintRequest, cups::Error> {
    let (base, event_id, row, badges) = {
        let current = CurrentEvent::lock(current);
        let Some(event_id) = current.event_id() else {
            return Err(cups::Error::new("no event"));
        };
        let base = current.base_dir();
        let row = current
            .full_row(&event_id, &job.id)
            .map_err(|err| cups::Error::new(err.to_string()))?;
        let badges = current.badges();
        (base, event_id, row.value, badges)
    };
    let Some(raw) = row else {
        return Err(cups::Error::new(format!("no registration {}", job.id)));
    };
    let value: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
    let event = base.join(&event_id);
    search.set_files(&base, &event_id);
    search.set_badges(badges);
    search.set_visitor(&job.id, fields_of(&value));
    let (transliterate, double_print) = settings_flags(&event.join("settings.json"));
    Ok(PrintRequest {
        id: job.id.clone(),
        category: job.category.unwrap_or_else(|| json_i64(&value, "category")),
        internet: json_flag(&value, "internet") || json_flag(&value, "is_internet"),
        zone: job.zone.clone().or_else(|| json_text(&value, "zone")),
        name: job.name.clone(),
        certificate: job
            .certificate
            .unwrap_or_else(|| json_flag(&value, "certificate") || json_flag(&value, "print_cert")),
        transliterate,
        double_print,
        rule: None,
        pixel_cap: None,
    })
}

fn fields_of(value: &serde_json::Value) -> HashMap<String, String> {
    let Some(object) = value.as_object() else {
        return HashMap::new();
    };
    let mut fields = HashMap::new();
    for (key, item) in object {
        let Some(text) = (match item {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }) else {
            continue;
        };
        fields.insert(key.clone(), text);
    }
    fields
}

fn json_i64(value: &serde_json::Value, key: &str) -> i64 {
    value
        .get(key)
        .and_then(|item| {
            item.as_i64()
                .or_else(|| item.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0)
}

fn json_flag(value: &serde_json::Value, key: &str) -> bool {
    match value.get(key) {
        Some(serde_json::Value::Bool(flag)) => *flag,
        Some(serde_json::Value::Number(number)) => number.as_i64() == Some(1),
        Some(serde_json::Value::String(text)) => text == "1" || text.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn json_text(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn settings_flags(path: &std::path::Path) -> (bool, bool) {
    let Ok(bytes) = std::fs::read(path) else {
        return (false, false);
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (false, false);
    };
    (
        json_flag(&value, "transliterate"),
        json_flag(&value, "double_print"),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ipp_tool::{Browse, Service};

    use super::load_request;
    use crate::registration_server::credentials::{CredentialFile, Credentials};
    use crate::registration_server::cups::Search;
    use crate::registration_server::current::CurrentEvent;
    use crate::registration_server::event_db::VisitorWrite;

    struct Quiet;

    impl Browse for Quiet {
        fn browse(&self) -> Vec<Service> {
            Vec::new()
        }
    }

    #[test]
    fn a_queued_job_reads_the_open_event() {
        let dir = std::env::temp_dir().join(format!(
            "rust-reg-print-job-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("AAA")).unwrap();
        let file = CredentialFile::new(dir.join("credentials.yml"));
        let mut credentials = Credentials::default();
        credentials.set_device_id("device-1").unwrap();
        credentials.set_base_url("https://kuprin.su/").unwrap();
        credentials
            .bind("AAA", "token-value", Some("Event".into()))
            .unwrap();
        file.store(&credentials).unwrap();
        let current = Mutex::new(CurrentEvent::new(&dir));
        CurrentEvent::lock(&current).open_bound(&file).unwrap();
        CurrentEvent::lock(&current)
            .write(
                "AAA",
                &VisitorWrite {
                    uid: "5".to_string(),
                    data: r#"{"uid":"5","name":"Ann","category":7,"internet":true,"zone":"hall"}"#
                        .to_string(),
                    name: "Ann".to_string(),
                    surname: String::new(),
                    c_name: String::new(),
                    category: 7,
                    ticket_status: 0,
                    gotsome: 0,
                    give_packet: 0,
                    added_ts: 1,
                    org_id: None,
                    email: String::new(),
                },
            )
            .unwrap();
        std::fs::write(
            dir.join("AAA").join("settings.json"),
            r#"{"double_print":true,"transliterate":false}"#,
        )
        .unwrap();
        let search = Search::start("http://127.0.0.1:9", Arc::new(Quiet));
        let request = load_request(
            &search,
            &current,
            &super::PrintJob {
                id: "5".to_string(),
                pages: 1,
                zone: Some("desk".to_string()),
                name: Some("HP".to_string()),
                certificate: None,
                category: None,
            },
        )
        .unwrap();
        assert_eq!(request.category, 7);
        assert!(request.internet);
        assert_eq!(request.zone.as_deref(), Some("desk"));
        assert_eq!(request.name.as_deref(), Some("HP"));
        assert!(request.double_print);
        assert!(!request.transliterate);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_print_keeps_the_template_cached_at_open() {
        let dir = std::env::temp_dir().join(format!(
            "rust-reg-print-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let badge = dir.join("AAA").join("badge").join("7");
        std::fs::create_dir_all(&badge).unwrap();
        let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/struct-to-pdf/fonts/LiberationSerif-Regular.ttf");
        std::fs::write(badge.join("page.yaml"), page_yaml(&font, "name")).unwrap();
        let file = CredentialFile::new(dir.join("credentials.yml"));
        let mut credentials = Credentials::default();
        credentials.set_device_id("device-1").unwrap();
        credentials.set_base_url("https://kuprin.su/").unwrap();
        credentials
            .bind("AAA", "token-value", Some("Event".into()))
            .unwrap();
        file.store(&credentials).unwrap();
        let current = Mutex::new(CurrentEvent::new(&dir));
        CurrentEvent::lock(&current).open_bound(&file).unwrap();
        std::fs::remove_dir_all(dir.join("AAA").join("badge")).unwrap();
        CurrentEvent::lock(&current)
            .write(
                "AAA",
                &VisitorWrite {
                    uid: "5".to_string(),
                    data: r#"{"uid":"5","name":"Ann","category":7}"#.to_string(),
                    name: "Ann".to_string(),
                    surname: String::new(),
                    c_name: String::new(),
                    category: 7,
                    ticket_status: 0,
                    gotsome: 0,
                    give_packet: 0,
                    added_ts: 1,
                    org_id: None,
                    email: String::new(),
                },
            )
            .unwrap();
        let search = Search::start("http://127.0.0.1:9", Arc::new(Quiet));
        let job = super::PrintJob {
            id: "5".to_string(),
            pages: 1,
            zone: None,
            name: None,
            certificate: None,
            category: None,
        };
        load_request(&search, &current, &job).unwrap();
        assert_eq!(
            search.prepared_fields(7).unwrap().unwrap(),
            vec!["name".to_string()]
        );
        let badge = dir.join("AAA").join("badge").join("7");
        std::fs::create_dir_all(&badge).unwrap();
        std::fs::write(badge.join("page.yaml"), page_yaml(&font, "company")).unwrap();
        CurrentEvent::lock(&current).store_page_file("AAA", 7);
        load_request(&search, &current, &job).unwrap();
        assert_eq!(
            search.prepared_fields(7).unwrap().unwrap(),
            vec!["company".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn page_yaml(font: &std::path::Path, field: &str) -> String {
        format!(
            r#"
page_size:
  width: 200
  height: 100
  units: points
  bleeds: {{top: 0, right: 0, bottom: 0, left: 0}}
resources:
  images: {{}}
  fonts:
    font1:
      name: font1
      embedded: true
      source_path: "{font}"
contents:
  - id: line
    ifSt: ""
    posX: 10
    posY: 10
    width: 160
    height: 30
    alignment:
      horizontal: left
      vertical: top
    padding: {{top: 0, right: 0, bottom: 0, left: 0}}
    template: true
    text:
      font: font1
      font_size: 12
      leading: 0
      line_height: 14
      preentered: "{{{{{field}}}}}"
      content: null
"#,
            font = font.display()
        )
    }
}
