mod cfg;
mod codes;
mod graphic;
mod layout;
mod look;
mod page;
mod replace;

pub use cfg::{decode_cfg, store_layout, CfgError};
pub use codes::render_qr;
pub use graphic::{png_graphic, render_graphic};
pub use layout::{load, store, Alignment, Area, AreaKind, Layout, LayoutError, Rect, TextPart};
pub use look::write_badge_png;
pub use page::{render_png, PageError};
pub use replace::{replace, replace_areas};
