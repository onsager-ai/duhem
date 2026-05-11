//! One-shot helper: parse a Verification Definition file and print its
//! canonical re-serialized form. Used to author round-trip-stable
//! fixtures.
//!
//! ```sh
//! cargo run -p duhem-schema --example canonicalize -- path/to/file.yml
//! ```

fn main() {
    let path = std::env::args().nth(1).expect("usage: canonicalize <path>");
    let src = std::fs::read_to_string(&path).expect("read");
    let parsed = duhem_schema::VerificationDefinition::from_yaml_str(&src).expect("parse");
    let out = parsed.to_yaml_string().expect("serialize");
    print!("{out}");
}
