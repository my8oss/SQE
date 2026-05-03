// === src/main.rs ===

mod convert;
mod items;
mod transcompiler;

use std::path::Path;
use std::process;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "sqe-core",
    version = env!("CARGO_PKG_VERSION"),
    about = "Scriptable Questionnaire Engine",
    after_help = "We value transparency and open-source collaboration. With that freedom comes responsibility: please test our tools in safe environments before production use. This product is provided as-is, without warranty of any kind."
)]
struct Args {
    /// Input .sqe file to compile
    #[arg(long, value_name = "FILE")]
    input: String,

    /// Output directory (defaults to ./out)
    #[arg(long, value_name = "DIR", default_value = "out")]
    output: String,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();

    // Basic validations with helpful messages
    let input_path = Path::new(&args.input);
    if !input_path.exists() {
        eprintln!("Input file does not exist: {}", input_path.display());
        eprintln!("Run with --help for usage information.");
        process::exit(2);
    }
    if !input_path.is_file() {
        eprintln!("Input path is not a file: {}", input_path.display());
        process::exit(2);
    }

    let out_dir = &args.output;

    // Compile and write pages
    let ast = match transcompiler::compile(input_path.to_str().unwrap()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to compile {}: {}", input_path.display(), e);
            process::exit(1);
        }
    };
    println!("Parsed AST:\n{:#?}", ast);

    if let Err(e) = convert::build_pages(&ast, out_dir) {
        eprintln!("Failed to write output to {}: {}", out_dir, e);
        process::exit(1);
    }
    println!("Wrote HTML files to {} (open {}/index.html)", out_dir, out_dir);
 
    // Print package & build metadata embedded at compile time
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!("Git commit: {}", env!("GIT_COMMIT"));
    println!("Build time (UTC): {}", env!("BUILD_TIME"));
 
    Ok(())
}
