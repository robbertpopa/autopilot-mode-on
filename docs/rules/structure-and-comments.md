<img src="../../assets/icon.svg" alt="autopilot-mode-on" width="48" align="right">

# Pack: structure, comments and docs

Sources: storytold (SC), craft-anything (CA), ArtCraft (AC), Google Engineering Practices (GG), Rust for Linux (RFL), our judgment (OJ).

## Structure

| id | rule | source | check | sev |
|---|---|---|---|---|
| ST-01 | A crate depends only on strictly lower layers; every crate is registered in the layer map; the map is checked by a script. | SC/CA `xtask layers` | script | high |
| ST-02 | Process spawning, filesystem writes and UI toolkit use are confined to named crates or modules; everything else calls them. Programs are launched with an argument list, absolute paths where possible, long text on stdin. | SC/CA golden rules, pitfalls | script + agent | medium |
| ST-03 | Library code does not print; it logs. `println!` and `eprintln!` only in tests and the CLI's output layer. | AC AGENTS.md; Clippy `print_stdout` | lint | low |
| ST-04 | Design: the change or code belongs where it is, interactions make sense, no abstraction, option or feature that nobody asked for. | GG "design", "complexity"; CA/SC "simple code" | agent | medium |
| ST-05 | Complexity: code that cannot be understood quickly is a finding. As hints only: files over about 800 lines, functions over about 80 lines, more than 4 levels of nesting, more than 6 parameters. | GG "complexity"; thresholds are OJ | agent + script for the hints | medium |
| ST-06 | Dead code: unused dependencies, unused public items, unreachable branches, commented-out code, feature flags that nothing enables. A branch no test reaches is a missing test, code that can be checked by reading, or dead code to delete. | OJ; storytold testing standard | tool (`cargo machete`) + lint (`dead_code`) + agent | low |
| ST-07 | Consistency: code matches its neighbours where the style guide is silent. | GG "consistency" | agent | nit |
| ST-08 | Names are clear and not too long; item names do not repeat the module or type namespace. | GG "naming"; RFL | agent | nit |

## Comments and docs

| id | rule | source | check | sev |
|---|---|---|---|---|
| CM-01 | Comments explain **why**, not what. A comment that restates the code is a finding. | GG "comments"; CA/SC | agent | low |
| CM-02 | A guard, check or workaround that looks removable says what it prevents. | SC house-rules-core | agent | low |
| CM-03 | No history comments ("used to", "moved from", ticket narratives) and no comments that contradict the code. | OJ | script (a short list of phrases) + agent | low |
| CM-04 | Item documentation uses doc comments (`///`), not plain comments; the first doc paragraph is one sentence. | RFL | agent / script | nit |
| CM-05 | Documentation is updated when behaviour, build, test, usage or release changes. | GG "documentation" | agent (compare diff and docs) | medium |
| CM-06 | Instructions and docs are readable by everyone: no notes about one machine, no absolute personal paths, no links to files a reader cannot open. | SC house-rules-core | script (`C:\Users\`, `/home/`, private links) | low |
