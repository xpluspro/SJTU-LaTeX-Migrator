#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    sjtu_latex_migrator_lib::run();
}
