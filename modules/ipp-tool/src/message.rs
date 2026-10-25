use crate::record::Error;

/// The operations this tool sends. `ResumePrinter` is `Resume-Printer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Operation {
    PrintJob = 0x0002,
    CancelJob = 0x0008,
    GetJobAttributes = 0x0009,
    GetJobs = 0x000A,
    GetPrinterAttributes = 0x000B,
    HoldJob = 0x000C,
    ReleaseJob = 0x000D,
    ResumePrinter = 0x0011,
    SetPrinterAttributes = 0x0013,
}

impl Operation {
    pub fn from_id(id: u16) -> Option<Self> {
        Some(match id {
            0x0002 => Self::PrintJob,
            0x0008 => Self::CancelJob,
            0x0009 => Self::GetJobAttributes,
            0x000A => Self::GetJobs,
            0x000B => Self::GetPrinterAttributes,
            0x000C => Self::HoldJob,
            0x000D => Self::ReleaseJob,
            0x0011 => Self::ResumePrinter,
            0x0013 => Self::SetPrinterAttributes,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Enum(i32),
    Keyword(String),
    Uri(String),
    Text(String),
    Name(String),
    Charset(String),
    NaturalLanguage(String),
    Mime(String),
    OctetString(Vec<u8>),
    Resolution { x: i32, y: i32, units: u8 },
    Collection(Vec<(String, Value)>),
    NoValue,
}

impl Value {
    fn tag(&self) -> u8 {
        match self {
            Self::Integer(_) => 0x21,
            Self::Boolean(_) => 0x22,
            Self::Enum(_) => 0x23,
            Self::Keyword(_) => 0x44,
            Self::Uri(_) => 0x45,
            Self::Text(_) => 0x41,
            Self::Name(_) => 0x42,
            Self::Charset(_) => 0x47,
            Self::NaturalLanguage(_) => 0x48,
            Self::Mime(_) => 0x49,
            Self::OctetString(_) => 0x30,
            Self::Resolution { .. } => 0x32,
            Self::Collection(_) => 0x34,
            Self::NoValue => 0x13,
        }
    }

    pub fn text(&self) -> Option<String> {
        if let Some(value) = self.as_str() {
            return Some(value.to_string());
        }
        match self {
            Self::Integer(value) | Self::Enum(value) => Some(value.to_string()),
            Self::Boolean(value) => Some(if *value { "true" } else { "false" }.to_string()),
            Self::Resolution { x, y, units } => Some(resolution_label(*x, *y, *units)),
            Self::Collection(members) => collection_label(members),
            Self::OctetString(_) | Self::NoValue => None,
            Self::Keyword(_)
            | Self::Uri(_)
            | Self::Text(_)
            | Self::Name(_)
            | Self::Charset(_)
            | Self::NaturalLanguage(_)
            | Self::Mime(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Keyword(value)
            | Self::Uri(value)
            | Self::Text(value)
            | Self::Name(value)
            | Self::Charset(value)
            | Self::NaturalLanguage(value)
            | Self::Mime(value) => Some(value),
            _ => None,
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Integer(value) | Self::Enum(value) => {
                write_u16(out, 4);
                out.extend_from_slice(&value.to_be_bytes());
            }
            Self::Boolean(value) => {
                write_u16(out, 1);
                out.push(u8::from(*value));
            }
            Self::NoValue => write_u16(out, 0),
            Self::OctetString(value) => {
                write_u16(out, value.len() as u16);
                out.extend_from_slice(value);
            }
            Self::Resolution { x, y, units } => {
                write_u16(out, 9);
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
                out.push(*units);
            }
            Self::Collection(_) => write_u16(out, 0),
            Self::Keyword(value)
            | Self::Uri(value)
            | Self::Text(value)
            | Self::Name(value)
            | Self::Charset(value)
            | Self::NaturalLanguage(value)
            | Self::Mime(value) => {
                write_u16(out, value.len() as u16);
                out.extend_from_slice(value.as_bytes());
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub tag: u8,
    pub attributes: Vec<Attribute>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub version_major: u8,
    pub version_minor: u8,
    pub code: u16,
    pub request_id: i32,
    pub groups: Vec<Group>,
}

impl Message {
    pub fn operation(&self) -> Option<Operation> {
        Operation::from_id(self.code)
    }

    pub fn strings(&self, name: &str) -> Vec<String> {
        self.groups
            .iter()
            .flat_map(|group| &group.attributes)
            .find(|attribute| attribute.name == name)
            .map(|attribute| {
                attribute
                    .values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn first_str(&self, name: &str) -> Option<String> {
        self.strings(name).into_iter().next()
    }

    pub fn named(&self, name: &str) -> Option<&Attribute> {
        self.groups
            .iter()
            .flat_map(|group| &group.attributes)
            .find(|attribute| attribute.name == name)
    }

    pub fn texts(&self, name: &str) -> Vec<String> {
        self.named(name)
            .map(|attribute| attribute.values.iter().filter_map(Value::text).collect())
            .unwrap_or_default()
    }
}

const OPERATION_ATTRIBUTES: u8 = 0x01;
const PRINTER_ATTRIBUTES: u8 = 0x04;
const END_OF_ATTRIBUTES: u8 = 0x03;

/// Encode an IPP 2.0 request. Charset and language are written first.
/// `printer_attributes` is the printer group a write sends. An empty slice omits that group.
pub fn encode_request(
    operation: Operation,
    request_id: i32,
    operation_attributes: &[(&str, Vec<Value>)],
    printer_attributes: &[(&str, Vec<Value>)],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x02);
    out.push(0x00);
    write_u16(&mut out, operation as u16);
    out.extend_from_slice(&request_id.to_be_bytes());
    out.push(OPERATION_ATTRIBUTES);
    write_attribute(
        &mut out,
        "attributes-charset",
        &[Value::Charset("utf-8".to_string())],
    );
    write_attribute(
        &mut out,
        "attributes-natural-language",
        &[Value::NaturalLanguage("en".to_string())],
    );
    for (name, values) in operation_attributes {
        write_attribute(&mut out, name, values);
    }
    if !printer_attributes.is_empty() {
        out.push(PRINTER_ATTRIBUTES);
        for (name, values) in printer_attributes {
            write_attribute(&mut out, name, values);
        }
    }
    out.push(END_OF_ATTRIBUTES);
    out
}

/// A response with a status code. `0x0000` is success. A client-error code is `0x0400` or higher.
#[cfg(test)]
pub fn encode_response(status: u16, request_id: i32) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x02);
    out.push(0x00);
    write_u16(&mut out, status);
    out.extend_from_slice(&request_id.to_be_bytes());
    out.push(OPERATION_ATTRIBUTES);
    write_attribute(
        &mut out,
        "attributes-charset",
        &[Value::Charset("utf-8".to_string())],
    );
    write_attribute(
        &mut out,
        "attributes-natural-language",
        &[Value::NaturalLanguage("en".to_string())],
    );
    out.push(END_OF_ATTRIBUTES);
    out
}

pub fn decode(body: &[u8]) -> Result<Message, Error> {
    if body.len() < 8 {
        return Err(truncated(body.len()));
    }
    let mut reader = Reader { body, index: 8 };
    let mut groups = Vec::new();
    let mut current: Option<usize> = None;
    let mut skipping = false;
    loop {
        let tag = reader.byte()?;
        if tag == END_OF_ATTRIBUTES {
            break;
        }
        if tag < 0x10 {
            groups.push(Group {
                tag,
                attributes: Vec::new(),
            });
            current = Some(groups.len() - 1);
            skipping = false;
            continue;
        }
        if tag == 0x34 {
            let name_len = reader.u16()? as usize;
            let name = reader.text(name_len)?;
            let value_len = reader.u16()? as usize;
            let _ = reader.bytes(value_len)?;
            let value = Value::Collection(read_members(&mut reader)?);
            push_value(&mut groups, current, &mut skipping, name, value)?;
            continue;
        }
        let name_len = reader.u16()? as usize;
        let name = reader.text(name_len)?;
        let value_len = reader.u16()? as usize;
        let raw = reader.bytes(value_len)?;
        if !known_tag(tag) {
            skipping = !name.is_empty() || skipping;
            continue;
        }
        let value = parse_value(tag, raw)?;
        push_value(&mut groups, current, &mut skipping, name, value)?;
    }
    Ok(Message {
        version_major: body[0],
        version_minor: body[1],
        code: u16::from_be_bytes([body[2], body[3]]),
        request_id: i32::from_be_bytes([body[4], body[5], body[6], body[7]]),
        groups,
    })
}

fn known_tag(tag: u8) -> bool {
    matches!(
        tag,
        0x10 | 0x12
            | 0x13
            | 0x21
            | 0x22
            | 0x23
            | 0x30
            | 0x32
            | 0x41
            | 0x42
            | 0x44
            | 0x45
            | 0x47
            | 0x48
            | 0x49
    )
}

fn parse_value(tag: u8, raw: &[u8]) -> Result<Value, Error> {
    let text = |raw: &[u8]| -> Result<String, Error> {
        String::from_utf8(raw.to_vec()).map_err(|_| Error::new("IPP attribute is not UTF-8"))
    };
    Ok(match tag {
        0x21 | 0x23 => {
            if raw.len() != 4 {
                return Err(Error::new(format!("IPP integer length is {}", raw.len())));
            }
            let value = i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
            if tag == 0x21 {
                Value::Integer(value)
            } else {
                Value::Enum(value)
            }
        }
        0x22 => Value::Boolean(raw.first().copied().unwrap_or(0) != 0),
        0x30 => Value::OctetString(raw.to_vec()),
        0x32 => {
            if raw.len() != 9 {
                return Err(Error::new(format!(
                    "IPP resolution length is {}",
                    raw.len()
                )));
            }
            Value::Resolution {
                x: i32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]),
                y: i32::from_be_bytes([raw[4], raw[5], raw[6], raw[7]]),
                units: raw[8],
            }
        }
        0x41 => Value::Text(text(raw)?),
        0x42 => Value::Name(text(raw)?),
        0x44 => Value::Keyword(text(raw)?),
        0x45 => Value::Uri(text(raw)?),
        0x47 => Value::Charset(text(raw)?),
        0x48 => Value::NaturalLanguage(text(raw)?),
        0x49 => Value::Mime(text(raw)?),
        0x10 | 0x12 | 0x13 => Value::NoValue,
        _ => return Err(Error::new(format!("IPP tag {tag:#04x} is unknown"))),
    })
}

fn push_value(
    groups: &mut Vec<Group>,
    current: Option<usize>,
    skipping: &mut bool,
    name: String,
    value: Value,
) -> Result<(), Error> {
    if name.is_empty() {
        if *skipping {
            return Ok(());
        }
        let group = current.ok_or_else(|| Error::new("IPP value has no attribute group"))?;
        let attribute = groups[group]
            .attributes
            .last_mut()
            .ok_or_else(|| Error::new("IPP additional value has no attribute"))?;
        attribute.values.push(value);
        return Ok(());
    }
    *skipping = false;
    let group = current.ok_or_else(|| Error::new("IPP attribute has no group"))?;
    groups[group].attributes.push(Attribute {
        name,
        values: vec![value],
    });
    Ok(())
}

fn read_members(reader: &mut Reader<'_>) -> Result<Vec<(String, Value)>, Error> {
    let mut members = Vec::new();
    loop {
        let tag = reader.byte()?;
        if tag == 0x37 {
            let name_len = reader.u16()? as usize;
            let _ = reader.bytes(name_len)?;
            let value_len = reader.u16()? as usize;
            let _ = reader.bytes(value_len)?;
            return Ok(members);
        }
        if tag != 0x4A {
            return Err(Error::new(format!("IPP collection member tag {tag:#04x}")));
        }
        let name_len = reader.u16()? as usize;
        let _ = reader.bytes(name_len)?;
        let value_len = reader.u16()? as usize;
        let member = reader.text(value_len)?;
        let value_tag = reader.byte()?;
        let value = read_tagged(reader, value_tag)?;
        members.push((member, value));
    }
}

fn read_tagged(reader: &mut Reader<'_>, tag: u8) -> Result<Value, Error> {
    if tag == 0x34 {
        let name_len = reader.u16()? as usize;
        let _ = reader.bytes(name_len)?;
        let value_len = reader.u16()? as usize;
        let _ = reader.bytes(value_len)?;
        return Ok(Value::Collection(read_members(reader)?));
    }
    let name_len = reader.u16()? as usize;
    let _ = reader.bytes(name_len)?;
    let value_len = reader.u16()? as usize;
    let raw = reader.bytes(value_len)?;
    parse_value(tag, raw)
}

fn resolution_label(x: i32, y: i32, units: u8) -> String {
    let unit = if units == 4 { "dpc" } else { "dpi" };
    if x == y {
        format!("{x}{unit}")
    } else {
        format!("{x}x{y}{unit}")
    }
}

fn collection_label(members: &[(String, Value)]) -> Option<String> {
    if let Some(media) = members.iter().find_map(|(name, value)| {
        if name == "media" {
            value.as_str().map(str::to_string)
        } else {
            None
        }
    }) {
        return Some(media);
    }
    let parts: Vec<String> = members
        .iter()
        .filter_map(|(_, value)| value.text())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn write_attribute(out: &mut Vec<u8>, name: &str, values: &[Value]) {
    for (index, value) in values.iter().enumerate() {
        out.push(value.tag());
        if index == 0 {
            write_u16(out, name.len() as u16);
            out.extend_from_slice(name.as_bytes());
        } else {
            write_u16(out, 0);
        }
        value.write(out);
    }
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn truncated(length: usize) -> Error {
    Error::new(format!("truncated IPP message, length {length}"))
}

struct Reader<'a> {
    body: &'a [u8],
    index: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, Error> {
        let bytes = self.bytes(1)?;
        Ok(bytes[0])
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn bytes(&mut self, len: usize) -> Result<&[u8], Error> {
        let end = self
            .index
            .checked_add(len)
            .filter(|end| *end <= self.body.len())
            .ok_or_else(|| truncated(self.body.len()))?;
        let slice = &self.body[self.index..end];
        self.index = end;
        Ok(slice)
    }

    fn text(&mut self, len: usize) -> Result<String, Error> {
        let bytes = self.bytes(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::new("IPP attribute name is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_roundtrips() {
        let uri = "ipp://192.0.2.10/ipp/print";
        let body = encode_request(
            Operation::GetPrinterAttributes,
            1,
            &[("printer-uri", vec![Value::Uri(uri.to_string())])],
            &[],
        );
        let message = decode(&body).unwrap();
        assert_eq!(message.version_major, 2);
        assert_eq!(message.version_minor, 0);
        assert_eq!(message.operation(), Some(Operation::GetPrinterAttributes));
        assert_eq!(message.first_str("printer-uri").as_deref(), Some(uri));
        assert_eq!(
            message.first_str("attributes-charset").as_deref(),
            Some("utf-8")
        );
        assert_eq!(
            message.first_str("attributes-natural-language").as_deref(),
            Some("en")
        );
    }

    #[test]
    fn a_truncated_body_is_an_error() {
        let body = [0x02, 0x00, 0x00];
        let err = decode(&body).unwrap_err();
        assert!(
            err.message().contains("length 3"),
            "error should name the length, got {err}"
        );
    }

    #[test]
    fn an_unknown_attribute_is_skipped() {
        let uri = "ipp://192.0.2.10/ipp/print";
        let mut body = encode_request(
            Operation::GetPrinterAttributes,
            1,
            &[("printer-uri", vec![Value::Uri(uri.to_string())])],
            &[],
        );
        assert_eq!(body.pop(), Some(END_OF_ATTRIBUTES));
        body.push(0x99);
        write_u16(&mut body, 1);
        body.push(b'x');
        write_u16(&mut body, 2);
        body.extend_from_slice(b"ab");
        body.push(END_OF_ATTRIBUTES);
        let message = decode(&body).unwrap();
        assert!(message.first_str("x").is_none());
        assert_eq!(message.first_str("printer-uri").as_deref(), Some(uri));
    }

    #[test]
    fn a_printer_attribute_roundtrips() {
        let body = encode_request(
            Operation::SetPrinterAttributes,
            7,
            &[(
                "printer-uri",
                vec![Value::Uri("ipp://192.0.2.10/ipp/print".to_string())],
            )],
            &[(
                "sides",
                vec![Value::Keyword("two-sided-long-edge".to_string())],
            )],
        );
        let message = decode(&body).unwrap();
        assert_eq!(message.operation(), Some(Operation::SetPrinterAttributes));
        assert_eq!(
            message.first_str("sides").as_deref(),
            Some("two-sided-long-edge")
        );
        assert!(message
            .groups
            .iter()
            .any(|group| group.tag == PRINTER_ATTRIBUTES));
    }
}
