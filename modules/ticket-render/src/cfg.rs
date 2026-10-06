use std::fs;
use std::path::Path;

use crate::layout::{store, Alignment, Area, AreaKind, Layout, Rect, TextPart};

#[derive(Debug)]
pub struct CfgError {
    message: String,
}

impl CfgError {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for CfgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CfgError {}

fn err(message: impl Into<String>) -> CfgError {
    CfgError {
        message: message.into(),
    }
}

const TC_NULL: u8 = 0x70;
const TC_REFERENCE: u8 = 0x71;
const TC_CLASSDESC: u8 = 0x72;
const TC_OBJECT: u8 = 0x73;
const TC_STRING: u8 = 0x74;
const TC_BLOCKDATA: u8 = 0x77;
const TC_ENDBLOCKDATA: u8 = 0x78;
const TC_BLOCKDATALONG: u8 = 0x7a;
const TC_LONGSTRING: u8 = 0x7c;
const BASE_HANDLE: u32 = 0x7e0000;
const HOLDER: &str = "com.nn.ticketrender.OutputAreasHolder";
const AREA: &str = "com.nn.ticketrender.OutputArea";

#[derive(Clone)]
struct ClassDesc {
    name: String,
    suid: i64,
    flags: u8,
}

enum Handle {
    Class(ClassDesc),
    Other,
}

enum Value {
    Null,
    Class(ClassDesc),
    Other,
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
    handles: Vec<Handle>,
}

pub fn decode_cfg(bytes: &[u8]) -> Result<Layout, CfgError> {
    let mut cur = Cursor {
        data: bytes,
        at: 0,
        handles: Vec::new(),
    };
    if cur.u16()? != 0xaced {
        return Err(err("not a java object stream"));
    }
    let version = cur.u16()?;
    if version != 5 {
        return Err(err(format!("stream version {version}")));
    }
    let (class, chunks) = cur.read_object()?;
    expect_version(&class, HOLDER)?;
    let mut prim = Vec::new();
    let mut areas = Vec::new();
    for chunk in chunks {
        match chunk {
            Chunk::Bytes(bytes) => prim.extend(bytes),
            Chunk::Object(class, inner) => {
                expect_version(&class, AREA)?;
                areas.push(decode_area(&inner)?);
            }
        }
    }
    let mut body = Bytes::new(&prim);
    let width = body.i32()?;
    let height = body.i32()?;
    let dots_per_point = body.f32()?;
    let count = body.i32()?;
    body.finish()?;
    if count < 0 || areas.len() != count as usize {
        return Err(err(format!(
            "area count {count} does not match {}",
            areas.len()
        )));
    }
    Ok(Layout {
        width,
        height,
        dots_per_point,
        areas,
    })
}

pub fn store_layout(dir: &Path, cfg_bytes: &[u8]) -> Result<(), CfgError> {
    let layout = decode_cfg(cfg_bytes)?;
    let json = store(&layout).map_err(|err| CfgError {
        message: err.message().to_string(),
    })?;
    fs::write(dir.join("layout.json"), json).map_err(|err| CfgError {
        message: err.to_string(),
    })?;
    Ok(())
}

fn expect_version(class: &ClassDesc, name: &str) -> Result<(), CfgError> {
    if class.name != name {
        return Err(err(format!("class {}", class.name)));
    }
    if class.suid != 5 {
        return Err(err(format!("serialVersionUID {}", class.suid)));
    }
    Ok(())
}

fn decode_area(chunks: &[Chunk]) -> Result<Area, CfgError> {
    let mut prim = Vec::new();
    for chunk in chunks {
        match chunk {
            Chunk::Bytes(bytes) => prim.extend(bytes),
            Chunk::Object(nested, _) => {
                return Err(err(format!("nested {}", nested.name)));
            }
        }
    }
    let mut body = Bytes::new(&prim);
    let rect = Rect {
        start_x: body.f64()?,
        start_y: body.f64()?,
        end_x: body.f64()?,
        end_y: body.f64()?,
    };
    let alignment = Alignment {
        x: body.i32()?,
        y: body.i32()?,
    };
    let rotation = body.i32()?;
    let color = body.i32()?;
    let text = body.utf()?;
    let condition = body.utf()?;
    let kind = body.i32()?;
    let area_kind = match kind {
        0 => body.text_kind()?,
        1 => AreaKind::Barcode {
            barcode_type: body.utf()?,
        },
        2 => {
            body.skip_text_kind()?;
            AreaKind::Photo
        }
        other => return Err(err(format!("unknown area type {other}"))),
    };
    body.finish()?;
    Ok(Area {
        rect,
        alignment,
        rotation,
        color,
        text,
        condition,
        kind: area_kind,
    })
}

struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], CfgError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| err("truncated badge.cfg"))?;
        let slice = &self.data[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn i32(&mut self) -> Result<i32, CfgError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().unwrap();
        Ok(i32::from_be_bytes(bytes))
    }

    fn f32(&mut self) -> Result<f32, CfgError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().unwrap();
        Ok(f32::from_be_bytes(bytes))
    }

    fn f64(&mut self) -> Result<f64, CfgError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().unwrap();
        Ok(f64::from_be_bytes(bytes))
    }

    fn utf(&mut self) -> Result<String, CfgError> {
        let len = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| err("text is not utf-8"))
    }

    fn text_kind(&mut self) -> Result<AreaKind, CfgError> {
        let (font_size, families, weight, style, parts) = self.text_fields()?;
        Ok(AreaKind::Text {
            font_size,
            families,
            weight,
            style,
            parts,
        })
    }

    fn skip_text_kind(&mut self) -> Result<(), CfgError> {
        self.text_fields()?;
        Ok(())
    }

    fn text_fields(
        &mut self,
    ) -> Result<(f32, Vec<String>, String, String, Vec<TextPart>), CfgError> {
        let font_size = self.f32()?;
        let families = self.strings()?;
        let weight = self.utf()?;
        let style = self.utf()?;
        let part_count = self.i32()?;
        if part_count < 0 {
            return Err(err(format!("text part count {part_count}")));
        }
        let mut parts = Vec::with_capacity(part_count as usize);
        for _ in 0..part_count {
            let text = self.utf()?;
            let is_break = self.take(1)?[0] != 0;
            let color = self.i32()?;
            let font_size = self.f32()?;
            // The writer stores the area's family count here, then that many names.
            let families = self.strings()?;
            let weight = self.utf()?;
            let style = self.utf()?;
            parts.push(TextPart {
                text,
                is_break,
                color,
                font_size,
                families,
                weight,
                style,
            });
        }
        Ok((font_size, families, weight, style, parts))
    }

    fn strings(&mut self) -> Result<Vec<String>, CfgError> {
        let count = self.i32()?;
        if count < 0 {
            return Err(err(format!("font family count {count}")));
        }
        let mut names = Vec::with_capacity(count as usize);
        for _ in 0..count {
            names.push(self.utf()?);
        }
        Ok(names)
    }

    fn finish(&self) -> Result<(), CfgError> {
        if self.at != self.data.len() {
            return Err(err("trailing bytes in badge.cfg"));
        }
        Ok(())
    }
}

enum Chunk {
    Bytes(Vec<u8>),
    Object(ClassDesc, Vec<Chunk>),
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, CfgError> {
        let byte = *self
            .data
            .get(self.at)
            .ok_or_else(|| err("truncated badge.cfg"))?;
        self.at += 1;
        Ok(byte)
    }

    fn u16(&mut self) -> Result<u16, CfgError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| err("truncated badge.cfg"))?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, CfgError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| err("truncated badge.cfg"))?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], CfgError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| err("truncated badge.cfg"))?;
        let slice = &self.data[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn utf_raw(&mut self) -> Result<String, CfgError> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| err("text is not utf-8"))
    }

    fn alloc(&mut self, handle: Handle) -> usize {
        self.handles.push(handle);
        self.handles.len() - 1
    }

    fn read_object(&mut self) -> Result<(ClassDesc, Vec<Chunk>), CfgError> {
        if self.u8()? != TC_OBJECT {
            return Err(err("expected an object"));
        }
        let class = match self.read_value()? {
            Value::Class(class) => class,
            _ => return Err(err("object has no class")),
        };
        self.alloc(Handle::Other);
        let chunks = self.read_custom(&class)?;
        Ok((class, chunks))
    }

    fn read_value(&mut self) -> Result<Value, CfgError> {
        match self.u8()? {
            TC_NULL => Ok(Value::Null),
            TC_REFERENCE => {
                let index = self.u32()?.wrapping_sub(BASE_HANDLE);
                match self.handles.get(index as usize) {
                    Some(Handle::Class(class)) => Ok(Value::Class(class.clone())),
                    Some(Handle::Other) => Ok(Value::Other),
                    None => Err(err(format!("bad reference {index}"))),
                }
            }
            TC_STRING => {
                self.alloc(Handle::Other);
                self.utf_raw()?;
                Ok(Value::Other)
            }
            TC_LONGSTRING => {
                self.alloc(Handle::Other);
                let high = self.u32()? as u64;
                let low = self.u32()? as u64;
                self.take((high << 32 | low) as usize)?;
                Ok(Value::Other)
            }
            TC_CLASSDESC => self.read_class().map(Value::Class),
            other => Err(err(format!("unexpected type {other:#x}"))),
        }
    }

    fn read_class(&mut self) -> Result<ClassDesc, CfgError> {
        let name = self.utf_raw()?;
        let suid = i64::from_be_bytes(self.take(8)?.try_into().unwrap());
        let index = self.alloc(Handle::Other);
        let flags = self.u8()?;
        let nfields = self.u16()?;
        for _ in 0..nfields {
            let kind = self.u8()? as char;
            self.utf_raw()?;
            if kind == 'L' || kind == '[' {
                self.read_value()?;
            }
        }
        self.skip_annotation()?;
        self.read_value()?;
        let class = ClassDesc { name, suid, flags };
        self.handles[index] = Handle::Class(class.clone());
        Ok(class)
    }

    fn skip_annotation(&mut self) -> Result<(), CfgError> {
        loop {
            match self.u8()? {
                TC_ENDBLOCKDATA => return Ok(()),
                TC_BLOCKDATA => {
                    let n = self.u8()? as usize;
                    self.take(n)?;
                }
                TC_BLOCKDATALONG => {
                    let n = self.u32()? as usize;
                    self.take(n)?;
                }
                other => return Err(err(format!("class annotation {other:#x}"))),
            }
        }
    }

    fn read_custom(&mut self, class: &ClassDesc) -> Result<Vec<Chunk>, CfgError> {
        if class.flags & 0x01 == 0 {
            return Err(err(format!("{} has no writeObject", class.name)));
        }
        let mut chunks = Vec::new();
        loop {
            match self.u8()? {
                TC_ENDBLOCKDATA => return Ok(chunks),
                TC_BLOCKDATA => {
                    let n = self.u8()? as usize;
                    chunks.push(Chunk::Bytes(self.take(n)?.to_vec()));
                }
                TC_BLOCKDATALONG => {
                    let n = self.u32()? as usize;
                    chunks.push(Chunk::Bytes(self.take(n)?.to_vec()));
                }
                TC_OBJECT => {
                    let nested = match self.read_value()? {
                        Value::Class(class) => class,
                        _ => return Err(err("object has no class")),
                    };
                    self.alloc(Handle::Other);
                    let inner = self.read_custom(&nested)?;
                    chunks.push(Chunk::Object(nested, inner));
                }
                other => return Err(err(format!("unexpected type {other:#x}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    const CFG: &[u8] = include_bytes!("../fixtures/badge.cfg");
    const RECORD: &str = include_str!("../fixtures/badge.txt");

    fn recorded() -> (i32, i32, usize, String) {
        let mut width = None;
        let mut height = None;
        let mut areas = None;
        let mut text = None;
        for line in RECORD.lines() {
            let (key, value) = line.split_once(' ').unwrap();
            match key {
                "width" => width = Some(value.parse().unwrap()),
                "height" => height = Some(value.parse().unwrap()),
                "areas" => areas = Some(value.parse().unwrap()),
                "text" => text = Some(value.to_string()),
                other => panic!("unknown record {other}"),
            }
        }
        (
            width.unwrap(),
            height.unwrap(),
            areas.unwrap(),
            text.unwrap(),
        )
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("ticket-render-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cfg_becomes_layout_json() {
        let (width, height, areas, text) = recorded();
        let layout = decode_cfg(CFG).unwrap();
        assert_eq!(layout.width, width);
        assert_eq!(layout.height, height);
        assert_eq!(layout.areas.len(), areas);
        assert_eq!(layout.areas[0].text, text);

        let dir = scratch("cfg");
        store_layout(&dir, CFG).unwrap();
        let path = dir.join("layout.json");
        let first = fs::read(&path).unwrap();
        let loaded = load(&first).unwrap();
        assert_eq!(loaded.width, width);
        assert_eq!(loaded.height, height);
        assert_eq!(loaded.areas.len(), areas);
        assert_eq!(loaded.areas[0].text, text);

        store_layout(&dir, CFG).unwrap();
        assert_eq!(fs::read(&path).unwrap(), first);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_version_is_rejected() {
        let name = HOLDER.as_bytes();
        let at = CFG
            .windows(name.len())
            .position(|window| window == name)
            .unwrap();
        let mut bad = CFG.to_vec();
        let suid = at + name.len();
        bad[suid + 7] = 4;

        let dir = scratch("version");
        let err = store_layout(&dir, &bad).unwrap_err();
        assert!(err.message().contains('4'), "message was {}", err.message());
        assert!(!dir.join("layout.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
