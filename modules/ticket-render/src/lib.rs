mod cfg;
mod codes;
mod graphic;
mod layout;
mod look;
mod page;
mod replace;

pub use cfg::{decode_cfg, store_layout, CfgError};
pub use codes::render_qr;
pub use graphic::{png_graphic, render_graphic, render_graphic_prepared, rgba_graphic};
pub use layout::{load, store, Alignment, Area, AreaKind, Layout, LayoutError, Rect, TextPart};
pub use look::write_badge_png;
pub use page::{
    capture_face, render_png, render_png_prepared, render_rgba_prepared, scale_background,
    PageError, PreparedFace,
};
pub use replace::{replace, replace_areas};
