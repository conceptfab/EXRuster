Bezpieczny (tylko najnowsze zgodne z Cargo.toml):

( cargo outdated -V >NUL 2>&1 || cargo install cargo-outdated --locked ) && cargo update && cargo outdated -R && cargo outdated


Pełny (podbijanie także MAJOR w Cargo.toml):

( cargo outdated -V >NUL 2>&1 || cargo install cargo-outdated --locked ) && ( cargo upgrade -V >NUL 2>&1 || cargo install cargo-edit --locked --force ) && cargo update && cargo outdated && ( cargo upgrade --incompatible || cargo upgrade --workspace || cargo upgrade --incompatible || cargo upgrade ) && cargo update && cargo build --release && cargo test && cargo outdated