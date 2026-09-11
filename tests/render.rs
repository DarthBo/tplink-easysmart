//! Render the views into an in-memory terminal and assert on what appears.
//! This is how the settings dialog gets checked without a real TTY.

use std::net::Ipv4Addr;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use tplink_easysmart::app::{Field, FormAction, SettingsForm};
use tplink_easysmart::model::SwitchInfo;
use tplink_easysmart::proto::client::Credentials;
use tplink_easysmart::proto::iface::Interface;
use tplink_easysmart::ui;

fn switch(desc: &str, ip: [u8; 4], dhcp: bool) -> SwitchInfo {
    SwitchInfo {
        model: "TL-SG108PE".into(),
        description: desc.into(),
        mac: [0xC0, 0x06, 0xC3, 0x1A, 0xCA, 0xA8],
        ip: ip.into(),
        netmask: Ipv4Addr::new(255, 255, 0, 0),
        gateway: Ipv4Addr::new(10, 0, 254, 254),
        firmware: "1.0.0 Build 20201030".into(),
        hardware: "TL-SG108PE 4.0".into(),
        dhcp,
        port_count: Some(8),
    }
}

fn iface() -> Interface {
    Interface {
        name: "eno1".into(),
        mac: [0x5C, 0x60, 0xBA, 0x3F, 0x73, 0x02],
        addr: Ipv4Addr::new(10, 0, 4, 1),
        netmask: Ipv4Addr::new(255, 255, 0, 0),
    }
}

/// Flatten the rendered buffer to plain text, one line per row.
fn render(width: u16, height: u16, f: impl FnOnce(&mut ratatui::Frame)) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| f(frame)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn table_lists_switches_and_marks_them() {
    let switches = vec![
        switch("Tibault", [10, 0, 4, 254], false),
        switch("Factory", [192, 168, 0, 1], true),
    ];
    // Narrow enough to prove the identifying columns are never squeezed.
    let text = render(72, 12, |frame| {
        ui::draw_table(frame, frame.area(), &switches, 0, &iface(), false);
    });
    println!("{text}");

    assert!(text.contains("Discovered switches"));
    assert!(text.contains("Tibault"));
    assert!(text.contains("10.0.4.254"));
    assert!(text.contains("192.168.0.1"));
    assert!(text.contains("C0:06:C3:1A:CA:A8"));
    // Detail columns live in the settings dialog, not the table.
    assert!(!text.contains("Firmware"));
    assert!(!text.contains("Gateway"));
}

#[test]
fn empty_table_explains_itself() {
    let text = render(60, 8, |frame| {
        ui::draw_table(frame, frame.area(), &[], 0, &iface(), false);
    });
    assert!(text.contains("No switches found"));
}

#[test]
fn settings_dialog_shows_current_values() {
    let form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials { username: "admin".into(), password: "secret".into() },
    );
    let text = render(100, 24, |frame| ui::draw_settings(frame, &form));
    println!("{text}");

    assert!(text.contains("TL-SG108PE"));
    assert!(text.contains("C0:06:C3:1A:CA:A8"));
    assert!(text.contains("10.0.4.254"));
    assert!(text.contains("255.255.0.0"));
    assert!(text.contains("10.0.254.254"));
    assert!(text.contains("admin"));
    // The password must never be rendered in the clear.
    assert!(!text.contains("secret"));
    assert!(text.contains("******"));
    assert!(text.contains("TL-SG108PE 4.0"));
    assert!(text.contains("1.0.0 Build 20201030"));
    assert!(text.contains("8 ports"));
    // The vendor dialog hides its save-config checkbox; we don't show one.
    assert!(!text.contains("Save to flash"));
}

#[test]
fn dhcp_form_hides_the_address_fields_from_tab_order() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], true),
        &Credentials::default(),
    );
    assert_eq!(form.field(), Field::Description);
    assert_eq!(form.on_key(key(KeyCode::Tab)), FormAction::Stay);
    assert_eq!(form.field(), Field::Dhcp);
    // With DHCP on, tabbing skips IP / mask / gateway entirely.
    form.on_key(key(KeyCode::Tab));
    assert_eq!(form.field(), Field::Username);
}

#[test]
fn turning_dhcp_off_reveals_the_address_fields() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], true),
        &Credentials::default(),
    );
    form.on_key(key(KeyCode::Tab)); // focus DHCP
    form.on_key(key(KeyCode::Char(' '))); // turn it off
    assert!(!form.dhcp);
    form.on_key(key(KeyCode::Tab));
    assert_eq!(form.field(), Field::Ip);
}

#[test]
fn typing_edits_the_focused_field() {
    let mut form = SettingsForm::from_switch(
        &switch("old", [10, 0, 4, 254], false),
        &Credentials::default(),
    );
    for _ in 0..3 {
        form.on_key(key(KeyCode::Backspace));
    }
    for c in "new1".chars() {
        form.on_key(key(KeyCode::Char(c)));
    }
    assert_eq!(form.description.value, "new1");
}

#[test]
fn enter_submits_and_escape_cancels() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials::default(),
    );
    assert_eq!(form.on_key(key(KeyCode::Enter)), FormAction::Submit);
    assert_eq!(form.on_key(key(KeyCode::Esc)), FormAction::Cancel);
}

#[test]
fn validation_rejects_bad_input_and_accepts_good() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials { username: "admin".into(), password: "admin".into() },
    );
    let (creds, settings) = form.to_settings().expect("valid form");
    assert_eq!(creds.username, "admin");
    assert_eq!(settings.ip, Ipv4Addr::new(10, 0, 4, 254));
    assert!(!settings.dhcp);

    form.ip.value = "999.1.1.1".into();
    assert!(form.to_settings().unwrap_err().contains("IP address"));

    form.ip.value = "10.0.4.254".into();
    form.description.value = "not valid!".into();
    assert!(form.to_settings().unwrap_err().contains("Description"));

    form.description.value = "ok".into();
    form.username.value = String::new();
    assert!(form.to_settings().unwrap_err().contains("Username"));
}

#[test]
fn dhcp_settings_zero_the_addresses() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials { username: "admin".into(), password: String::new() },
    );
    form.dhcp = true;
    let (_, settings) = form.to_settings().unwrap();
    assert!(settings.dhcp);
    assert!(settings.ip.is_unspecified());
    assert!(settings.netmask.is_unspecified());
    assert!(settings.gateway.is_unspecified());
}

#[test]
fn dialog_shows_apply_and_cancel_buttons() {
    let form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials::default(),
    );
    let text = render(100, 24, |frame| ui::draw_settings(frame, &form));
    assert!(text.contains("Apply"));
    assert!(text.contains("Cancel"));
}

#[test]
fn tab_order_ends_on_the_buttons() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials::default(),
    );
    // Description, DHCP, IP, mask, gateway, username, password.
    for _ in 0..7 {
        form.on_key(key(KeyCode::Tab));
    }
    assert_eq!(form.field(), Field::Apply);
    form.on_key(key(KeyCode::Tab));
    assert_eq!(form.field(), Field::Cancel);
}

#[test]
fn buttons_activate_on_enter_and_space() {
    let make = || {
        SettingsForm::from_switch(
            &switch("Tibault", [10, 0, 4, 254], false),
            &Credentials::default(),
        )
    };

    let mut form = make();
    for _ in 0..7 {
        form.on_key(key(KeyCode::Tab));
    }
    assert_eq!(form.field(), Field::Apply);
    assert_eq!(form.on_key(key(KeyCode::Char(' '))), FormAction::Submit);
    assert_eq!(form.on_key(key(KeyCode::Enter)), FormAction::Submit);

    // Cancel must not submit, however it is activated.
    let mut form = make();
    for _ in 0..8 {
        form.on_key(key(KeyCode::Tab));
    }
    assert_eq!(form.field(), Field::Cancel);
    assert_eq!(form.on_key(key(KeyCode::Enter)), FormAction::Cancel);
    assert_eq!(form.on_key(key(KeyCode::Char(' '))), FormAction::Cancel);
}

#[test]
fn space_on_cancel_does_not_type_into_a_field() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials::default(),
    );
    for _ in 0..8 {
        form.on_key(key(KeyCode::Tab));
    }
    form.on_key(key(KeyCode::Char(' ')));
    assert_eq!(form.description.value, "Tibault");
}

#[test]
fn arrows_move_between_the_two_buttons() {
    let mut form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials::default(),
    );
    for _ in 0..7 {
        form.on_key(key(KeyCode::Tab));
    }
    assert_eq!(form.field(), Field::Apply);
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.field(), Field::Cancel);
    form.on_key(key(KeyCode::Left));
    assert_eq!(form.field(), Field::Apply);
}

#[test]
fn read_only_rows_are_dimmed() {
    use ratatui::style::Color;

    let form = SettingsForm::from_switch(
        &switch("Tibault", [10, 0, 4, 254], false),
        &Credentials::default(),
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| ui::draw_settings(frame, &form)).unwrap();
    let buffer = terminal.backend().buffer().clone();

    // Locate a row by its text, then check every glyph on it is dim -- the
    // value as well as the label, so it cannot be mistaken for an input.
    let colour_of_row_containing = |needle: &str| -> Vec<Color> {
        for y in 0..buffer.area.height {
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if line.contains(needle) {
                return (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .enumerate()
                    // Skip blanks and the dialog's own border glyphs.
                    .filter(|(_, sym)| {
                        !sym.trim().is_empty() && !sym.chars().all(|c| ('\u{2500}'..='\u{257F}').contains(&c))
                    })
                    .map(|(x, _)| buffer[(x as u16, y)].style().fg.unwrap_or(Color::Reset))
                    .collect();
            }
        }
        panic!("no row containing {needle:?}");
    };

    for needle in ["MAC Address", "Hardware Version", "Firmware Version"] {
        let colours = colour_of_row_containing(needle);
        assert!(!colours.is_empty(), "{needle} row was blank");
        assert!(
            colours.iter().all(|c| *c == Color::DarkGray),
            "{needle} row should be entirely dim, got {colours:?}"
        );
    }

    // An editable field must not be dim, or the distinction is meaningless.
    let editable = colour_of_row_containing("Device Description");
    assert!(editable.iter().any(|c| *c != Color::DarkGray));
}

#[test]
fn confirm_box_is_not_clipped() {
    for dhcp in [false, true] {
        let form = SettingsForm::from_switch(
            &switch("Tibault", [10, 0, 4, 254], dhcp),
            &Credentials::default(),
        );
        let text = render(120, 30, |frame| ui::draw_confirm(frame, &form));
        println!("--- dhcp={dhcp}\n{text}");

        // The instruction has to survive whole; it was previously wrapping
        // onto a line the box had no room for.
        assert!(text.contains("Press y to confirm, or esc to cancel."));
        assert!(text.contains("applies this immediately"));
        // And the switch must still be identifiable.
        assert!(text.contains("TL-SG108PE"));
        assert!(text.contains("C0:06:C3:1A:CA:A8"));

        // Every rendered line must sit inside the border box.
        let bordered: Vec<&str> = text
            .lines()
            .filter(|l| l.contains('│') || l.contains('┌') || l.contains('└'))
            .collect();
        let body: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.contains('│') && !l.contains('┌') && !l.contains('└'))
            .collect();
        assert!(!bordered.is_empty(), "box did not render");
        assert!(body.is_empty(), "text escaped the box: {body:?}");
    }
}
