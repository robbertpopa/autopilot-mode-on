//! The deny hook as a program of its own, so it can run where the full command
//! line cannot: inside the Linux sandbox, built there from this source. It is
//! the same decision as `autopilot hook`.

use std::io::{self, Read, Write};

fn main() -> io::Result<()> {
    let mut input = Vec::new();
    io::stdin().lock().take(1 << 20).read_to_end(&mut input)?;
    let verdict = autopilot_policy::hook::decide(&input);
    writeln!(
        io::stdout().lock(),
        "{}",
        autopilot_policy::hook::response(&verdict)
    )
}
