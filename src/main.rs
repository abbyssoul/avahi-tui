//! Thin binary entry point. All logic lives in the `avahi_tui` library so it can
//! be reused and fuzzed; see [`avahi_tui::run`].

fn main() -> color_eyre::eyre::Result<()> {
    avahi_tui::run()
}
