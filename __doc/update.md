Bezpieczny (tylko najnowsze zgodne z Cargo.toml):

( cargo outdated -V >NUL 2>&1 || cargo install cargo-outdated --locked ) && cargo update && cargo outdated -R && cargo outdated


Pełny (podbijanie także MAJOR w Cargo.toml):

cargo install cargo-edit --force --locked && cargo update && ( cargo upgrade --workspace --incompatible || cargo upgrade --workspace || cargo upgrade --incompatible || cargo upgrade ) && cargo update && cargo build --release && cargo test && cargo outdated


Jeśli flaga --incompatible też byłaby niedostępna u Ciebie, użyj po prostu:

cargo upgrade --all