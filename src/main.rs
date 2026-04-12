#[tokio::main]
async fn main() {
    if let Err(error) = fpy::run().await {
        eprintln!("error: {error}");

        for (index, cause) in error.chain().skip(1).enumerate() {
            if index == 0 {
                eprintln!();
                eprintln!("Caused by:");
            }
            eprintln!("  {}. {cause}", index + 1);
        }

        std::process::exit(1);
    }
}
