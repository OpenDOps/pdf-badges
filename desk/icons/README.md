# reg-server icons (Linux deb / pixmaps)

These PNGs are the **macOS menu-bar mark** (system `pencil` + person), recolored
to brand blue `#1f6feb` so they stay visible in the GNOME app grid.

Regenerate on a Mac after changing the tray drawing:

```bash
cargo build --manifest-path desk/Cargo.toml
bin=desk/target/debug/rust-reg-desk
$bin --write-icon /tmp/mac-src.png 256
magick /tmp/mac-src.png -alpha extract /tmp/alpha.png
magick -size 256x256 xc:'#1f6feb' /tmp/alpha.png -alpha off \
  -compose CopyOpacity -composite desk/icons/rust-reg-256.png
for edge in 22 24 32 48 64 128 256; do
  magick desk/icons/rust-reg-256.png -resize "${edge}x${edge}" \
    "desk/icons/rust-reg-${edge}.png"
done
cp -f desk/icons/rust-reg-128.png desk/icons/rust-reg.png
```

`scripts/release/pack.sh` installs these into the `.deb` when present.
