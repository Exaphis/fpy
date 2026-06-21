use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        bail!(
            "usage: fpy-display-fixture <scenario|sequence> <name> --width <cols> --height <rows>"
        );
    };
    if command != "scenario" && command != "sequence" {
        bail!("expected `scenario` or `sequence`, got `{command}`");
    }
    let name = args.next().context(
        "usage: fpy-display-fixture <scenario|sequence> <name> --width <cols> --height <rows>",
    )?;
    let mut width = 80u16;
    let mut height = 24u16;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--width" => {
                width = args
                    .next()
                    .context("--width requires a value")?
                    .parse()
                    .context("--width must be a positive integer")?;
            }
            "--height" => {
                height = args
                    .next()
                    .context("--height requires a value")?
                    .parse()
                    .context("--height must be a positive integer")?;
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let output = match command.as_str() {
        "scenario" => fpy::display_fixture_json(&name, width, height)?,
        "sequence" => fpy::display_fixture_sequence_json(&name, width, height)?,
        _ => unreachable!("command was validated above"),
    };
    println!("{output}");
    Ok(())
}
