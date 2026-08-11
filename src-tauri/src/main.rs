use std::{env, path::Path};

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().map(String::as_str) == Some("--export") {
        if arguments.len() < 2
            || arguments.len() > 3
            || (arguments.len() == 3 && arguments[2] != "--force")
        {
            eprintln!("Usage: pbi-lens --export <report.pbix|report.pbit> [--force]");
            std::process::exit(2);
        }
        match pbi_lens_lib::export::export_report(
            Path::new(&arguments[1]),
            arguments.get(2).map(String::as_str) == Some("--force"),
        ) {
            Ok(output) => {
                println!("{}", output.display());
                return;
            }
            Err(error) => {
                eprintln!("Export failed: {error}");
                std::process::exit(1);
            }
        }
    }
    if arguments.first().map(String::as_str) == Some("--help") {
        println!("PBI Lens\n\n  pbi-lens --export <report.pbix|report.pbit> [--force]\n\nExports a machine-readable <report>.pbilens.json anatomy beside the source file.");
        return;
    }
    pbi_lens_lib::run();
}
