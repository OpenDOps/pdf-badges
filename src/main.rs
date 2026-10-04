use clap::Parser;
use std::path::PathBuf;

use rust_reg::modules::structured_analyzer::StructuredPdfAnalyzer;
use rust_reg::struct_to_pdf::{load_page, render_page, save_pdf};

#[derive(Parser)]
#[command(name = "render-page")]
struct RenderPageArgs {
    /// Page YAML or JSON file
    page: PathBuf,

    /// Output PDF path
    #[arg(short, long)]
    output: PathBuf,
}

#[derive(Parser)]
#[command(name = "rust-reg")]
#[command(about = "A CLI tool to analyze PDF structure and extract structured data")]
struct Args {
    /// Path to the input PDF file
    input: PathBuf,

    /// Output directory for extracted files and analysis results
    #[arg(short, long, default_value = "output")]
    output: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        Some("render-page") => return render_page_cli(),
        Some("serve") => return serve_cli(),
        Some("registration-server") => return registration_server_cli(),
        _ => {}
    }
    let args = Args::parse();

    println!("PDF Structure Analysis for: {:?}", args.input);
    println!("==========================================");

    // Create output directory
    std::fs::create_dir_all(&args.output)?;

    // Analyze the PDF and extract structured data
    println!("Analyzing PDF structure...");
    let analysis = StructuredPdfAnalyzer::analyze_pdf(&args.input)?;

    // Print summary
    println!("Analysis complete!");
    println!("Pages: {}", analysis.metadata.page_count);
    println!(
        "Total images: {}",
        analysis
            .pages
            .iter()
            .map(|p| p.resources.images.len())
            .sum::<usize>()
    );
    println!(
        "Total fonts: {}",
        analysis
            .pages
            .iter()
            .map(|p| p.resources.fonts.len())
            .sum::<usize>()
    );

    // Save results to JSON and YAML
    let json_path = args.output.join("analysis.json");
    let yaml_path = args.output.join("analysis.yaml");

    println!("Saving results to JSON: {:?}", json_path);
    StructuredPdfAnalyzer::save_to_json(&analysis, json_path.to_str().unwrap())?;

    println!("Saving results to YAML: {:?}", yaml_path);
    StructuredPdfAnalyzer::save_to_yaml(&analysis, yaml_path.to_str().unwrap())?;

    println!(
        "Analysis complete! Check the '{}' directory for results.",
        args.output.display()
    );

    Ok(())
}

#[derive(Parser)]
#[command(name = "serve")]
struct ServeArgs {
    /// Address to listen on
    #[arg(long, default_value = "0.0.0.0:50052")]
    listen: String,
}

fn serve_cli() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = std::env::args();
    let program = raw.next().unwrap_or_else(|| "rust-reg".into());
    let _command = raw.next();
    let args = match ServeArgs::try_parse_from(std::iter::once(program).chain(raw)) {
        Ok(args) => args,
        Err(err) => err.exit(),
    };
    let listen = args.listen.parse()?;
    let runtime = tokio::runtime::Runtime::new()?;
    runtime
        .block_on(rust_reg::pdf_server::serve(listen))
        .map_err(|err| err.to_string())?;
    Ok(())
}

#[derive(Parser)]
#[command(name = "registration-server")]
struct RegistrationServerArgs {
    /// HTTP and WebSocket listen address
    #[arg(long, default_value = "0.0.0.0:8080")]
    http: String,

    /// Seconds between remote sync cycles
    #[arg(long, default_value_t = 60)]
    sync_every: u64,

    /// Remote calls that may wait on a response at once. Replaces the compiled SYNC_IN_FLIGHT for this process
    #[arg(long, default_value_t = rust_reg::registration_server::SYNC_IN_FLIGHT)]
    sync_in_flight: usize,

    /// Remote endpoint to download. Repeat for each endpoint.
    #[arg(long = "endpoint")]
    endpoint: Vec<String>,

    /// Built admin files (`npm run build` in registration-admin)
    #[arg(long, default_value = "registration-admin/dist")]
    dist: PathBuf,

    /// Credential document
    #[arg(long, default_value = "credentials.yml")]
    credentials: PathBuf,

    /// Debug lines for HTTP, login, events, select, sync, and print
    #[arg(long)]
    debug: bool,

    /// Log every ping of the internet server. Separate from --debug
    #[arg(long)]
    debug_ping: bool,
}

fn registration_server_cli() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = std::env::args();
    let program = raw.next().unwrap_or_else(|| "rust-reg".into());
    let _command = raw.next();
    let args = match RegistrationServerArgs::try_parse_from(std::iter::once(program).chain(raw)) {
        Ok(args) => args,
        Err(err) => err.exit(),
    };
    if args.sync_every == 0 {
        eprintln!("sync interval must be greater than zero");
        std::process::exit(1);
    }
    if args.sync_in_flight == 0 {
        eprintln!("sync in flight must be greater than zero");
        std::process::exit(1);
    }
    let remote_base = rust_reg::registration_server::remote_base_from_env()?;
    let config = rust_reg::registration_server::Config {
        http_listen: args.http.parse()?,
        endpoints: args.endpoint,
        sync_interval: std::time::Duration::from_secs(args.sync_every),
        sync_in_flight: args.sync_in_flight,
        remote_base: remote_base.into(),
        dist: args.dist,
        credentials_path: args.credentials,
        debug: args.debug,
        debug_ping: args.debug_ping,
    };
    rust_reg::registration_server::run(config).map_err(|err| err.to_string())?;
    Ok(())
}

fn render_page_cli() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = std::env::args();
    let program = raw.next().unwrap_or_else(|| "rust-reg".into());
    let _command = raw.next();
    let args = match RenderPageArgs::try_parse_from(std::iter::once(program).chain(raw)) {
        Ok(args) => args,
        Err(err) => err.exit(),
    };
    let page = match load_page(&args.page) {
        Ok(page) => page,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let base_dir = args.page.parent().unwrap_or(std::path::Path::new("."));
    match render_page(&page, base_dir) {
        Ok(bytes) => save_pdf(&bytes, &args.output)?,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
    Ok(())
}
