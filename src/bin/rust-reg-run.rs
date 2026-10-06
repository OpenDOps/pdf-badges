fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("version") => {
            println!("{}", rust_reg::registration_server::release_tag());
        }
        Some("--prefix") => {
            let Some(prefix) = args.next() else {
                usage();
            };
            if args.next().is_some() {
                usage();
            }
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
    eprintln!("usage: rust-reg-run version | rust-reg-run --prefix <prefix>");
    std::process::exit(1);
}
