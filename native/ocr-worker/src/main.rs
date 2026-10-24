#![allow(dead_code, unused_imports)]
#[path = "../../../src-tauri/src/proc.rs"]
mod proc;
#[path = "../../../src-tauri/src/rapidocr.rs"]
mod rapidocr;
mod offline_models {
    include!("../../../src-tauri/src/offline_models/core.rs");
    include!("ort.rs");
}
fn main() -> std::process::ExitCode {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(rapidocr::RAPIDOCR_WORKER_ARG)) {
        eprintln!("This helper is launched by Kivio for local OCR.");
        return std::process::ExitCode::from(2);
    }
    rapidocr::run_worker_entry(args)
}
