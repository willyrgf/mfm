use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use walkdir::WalkDir;

//TODO: need improve
fn main() {
    let path = Path::new(&env::var("OUT_DIR").unwrap()).join("res.rs");
    let mut file = BufWriter::new(File::create(&path).unwrap());

    let mut map = phf_codegen::Map::new();

    for dir_entry in WalkDir::new("res").into_iter().filter_map(|e| e.ok()) {
        let p = dir_entry.path();
        let path = p.to_str().unwrap().to_owned();
        if !dir_entry.file_type().is_file() {
            continue;
        }
        let file = std::fs::read_to_string(path.clone()).unwrap();
        map.entry(path.to_owned(), &format!("\"{}\"", file.as_str().escape_default().to_string().as_str()));
    }

    writeln!(
        &mut file,
            "static RES: phf::Map<&'static str, &'static str> = \n{};",
            map.build()
    ).unwrap();
}
