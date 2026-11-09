// The wrapper carries no release tag, so its SHA-256 stays the same across releases that leave its code alone.
fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--prefix") => {
            let Some(prefix) = args.next() else {
                usage();
            };
            if args.next().is_some() {
                usage();
            }
            rust_reg::registration_server::emit_upgrade_probe("rust-reg-run", "wrapper");
            if let Err(err) =
                rust_reg::registration_server::run_supervisor(std::path::Path::new(&prefix))
            {
                eprintln!("rust-reg-run: {err}");
                std::process::exit(1);
            }
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("usage: rust-reg-run --prefix <prefix>");
    std::process::exit(1);
}
