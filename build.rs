use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use walkdir::WalkDir;

fn main() {
    let path = Path::new(&env::var("OUT_DIR").unwrap()).join("res.rs");
    let mut file_dest = BufWriter::new(File::create(&path).unwrap());
    let mut map = phf_codegen::Map::new();

    for dir_entry in WalkDir::new("res").into_iter().filter_map(|dir| match dir {
        Ok(d) if d.file_type().is_file() => Some(d),
        Ok(_) => None,
        Err(_) => None,
    }) {
        let path = dir_entry.path().to_str().unwrap().to_owned();
        let file = std::fs::read_to_string(path.clone()).unwrap();
        map.entry(
            path.to_owned(),
            &format!("\"{}\"", file.as_str().escape_default()),
        );
    }

    writeln!(
        &mut file_dest,
        "static RES: phf::Map<&'static str, &'static str> = \n{};",
        map.build()
    )
    .unwrap();
}
