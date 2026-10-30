// Desktop only: the phone app's interface in a window, for working on it
// without a phone. Phones load the library through `mobile_entry_point`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sourcelight_mobile_lib::run()
}
