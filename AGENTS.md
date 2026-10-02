# fjx contributor notes

- Keep `fjx` a synchronous, single-binary Rust crate with no public library API.
- Runtime dependencies may only be `lexopt`, `ureq` with Rustls, `serde`, `serde_json`, and `rpassword`.
- Keep host, repository, raw-path, config, HTTP, and output rules inside their owning modules.
- Never print a token, follow an HTTP redirect, retry a write, or emit stdout before a command has a final result.
- Update `release.toml`, `Cargo.toml`, and the `fjx` entry in `Cargo.lock` together with `scripts/release.nu`.
- Before submitting, run `nu --no-config-file scripts/check.nu`. It runs the Cargo format, Clippy, test, doc, and Nu checks used by `.github/workflows/ci.yml`, ending with `nu --no-config-file scripts/release.nu validate`.
- Keep build instructions in `README.md` and release instructions in `docs/releasing.md` aligned with the checked-in scripts. Do not claim publication or native-platform coverage that CI does not provide.
