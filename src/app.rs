//! Application state and key handling.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::model::SwitchInfo;
use crate::proto::client::{Client, Credentials, IpSettings};

pub const DISCOVERY_WINDOW: Duration = Duration::from_millis(2500);
pub const REPLY_WINDOW: Duration = Duration::from_millis(1500);

/// A single-line text field with a cursor.
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
    pub max_len: usize,
    pub secret: bool,
}

impl TextInput {
    pub fn new(value: impl Into<String>, max_len: usize) -> Self {
        let value = value.into();
        TextInput { cursor: value.chars().count(), value, max_len, secret: false }
    }

    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    pub fn display(&self) -> String {
        if self.secret {
            "*".repeat(self.value.chars().count())
        } else {
            self.value.clone()
        }
    }

    fn byte_at(&self, index: usize) -> usize {
        self.value
            .char_indices()
            .nth(index)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    pub fn insert(&mut self, c: char) {
        if self.value.chars().count() >= self.max_len {
            return;
        }
        let at = self.byte_at(self.cursor);
        self.value.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let at = self.byte_at(self.cursor - 1);
        self.value.remove(at);
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor < self.value.chars().count() {
            let at = self.byte_at(self.cursor);
            self.value.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.value.chars().count());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.chars().count();
    }
}

/// Fields of the settings form, in tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Description,
    Dhcp,
    Ip,
    Netmask,
    Gateway,
    Username,
    Password,
    Apply,
    Cancel,
}

impl Field {
    pub const ORDER: [Field; 9] = [
        Field::Description,
        Field::Dhcp,
        Field::Ip,
        Field::Netmask,
        Field::Gateway,
        Field::Username,
        Field::Password,
        Field::Apply,
        Field::Cancel,
    ];

    /// The labelled rows, i.e. everything above the button row.
    pub const ROWS: usize = 7;

    pub fn label(self) -> &'static str {
        match self {
            Field::Description => "Device Description",
            Field::Dhcp => "DHCP Setting",
            Field::Ip => "IP Address",
            Field::Netmask => "Subnet Mask",
            Field::Gateway => "Default Gateway",
            Field::Username => "User Name",
            Field::Password => "Password",
            Field::Apply => "Apply",
            Field::Cancel => "Cancel",
        }
    }

    fn is_toggle(self) -> bool {
        matches!(self, Field::Dhcp)
    }

    pub fn is_button(self) -> bool {
        matches!(self, Field::Apply | Field::Cancel)
    }
}

#[derive(Clone)]
pub struct SettingsForm {
    pub mac: [u8; 6],
    pub title: String,
    /// Read-only detail shown above the editable fields, as the vendor
    /// dialog does.
    pub mac_display: String,
    pub hardware: String,
    pub firmware: String,
    pub port_count: Option<u8>,
    pub description: TextInput,
    pub dhcp: bool,
    pub ip: TextInput,
    pub netmask: TextInput,
    pub gateway: TextInput,
    pub username: TextInput,
    pub password: TextInput,
    pub focus: usize,
}

impl SettingsForm {
    pub fn from_switch(s: &SwitchInfo, creds: &Credentials) -> SettingsForm {
        SettingsForm {
            mac: s.mac,
            title: format!("IP Setting  -  {}", s.model),
            mac_display: s.mac_string(),
            hardware: s.hardware.clone(),
            firmware: s.firmware.clone(),
            port_count: s.port_count,
            description: TextInput::new(s.description.clone(), 32),
            dhcp: s.dhcp,
            ip: TextInput::new(s.ip.to_string(), 15),
            netmask: TextInput::new(s.netmask.to_string(), 15),
            gateway: TextInput::new(s.gateway.to_string(), 15),
            username: TextInput::new(creds.username.clone(), 16),
            password: TextInput::new(creds.password.clone(), 16).secret(),
            focus: 0,
        }
    }

    pub fn field(&self) -> Field {
        Field::ORDER[self.focus]
    }

    /// IP fields are inert while the switch is on DHCP, matching the utility.
    pub fn is_disabled(&self, field: Field) -> bool {
        self.dhcp && matches!(field, Field::Ip | Field::Netmask | Field::Gateway)
    }

    pub fn input_mut(&mut self, field: Field) -> Option<&mut TextInput> {
        match field {
            Field::Description => Some(&mut self.description),
            Field::Ip => Some(&mut self.ip),
            Field::Netmask => Some(&mut self.netmask),
            Field::Gateway => Some(&mut self.gateway),
            Field::Username => Some(&mut self.username),
            Field::Password => Some(&mut self.password),
            Field::Dhcp | Field::Apply | Field::Cancel => None,
        }
    }

    pub fn input(&self, field: Field) -> Option<&TextInput> {
        match field {
            Field::Description => Some(&self.description),
            Field::Ip => Some(&self.ip),
            Field::Netmask => Some(&self.netmask),
            Field::Gateway => Some(&self.gateway),
            Field::Username => Some(&self.username),
            Field::Password => Some(&self.password),
            Field::Dhcp | Field::Apply | Field::Cancel => None,
        }
    }

    pub fn toggle(&self, field: Field) -> bool {
        matches!(field, Field::Dhcp) && self.dhcp
    }

    /// Handle one keypress aimed at the form. Returns what the caller should
    /// do next; all field editing is contained here.
    pub fn on_key(&mut self, key: KeyEvent) -> FormAction {
        let field = self.field();
        match key.code {
            KeyCode::Esc => return FormAction::Cancel,
            // Enter submits from anywhere, except while sitting on Cancel.
            KeyCode::Enter | KeyCode::Char(' ') if field == Field::Cancel => {
                return FormAction::Cancel
            }
            KeyCode::Enter | KeyCode::Char(' ') if field == Field::Apply => {
                return FormAction::Submit
            }
            KeyCode::Enter => return FormAction::Submit,
            // Left/right moves between the two buttons rather than typing.
            KeyCode::Left if field == Field::Cancel => self.step_focus(-1),
            KeyCode::Right if field == Field::Apply => self.step_focus(1),
            KeyCode::Tab | KeyCode::Down => self.step_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.step_focus(-1),
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if field.is_toggle() => {
                if field == Field::Dhcp {
                    self.dhcp = !self.dhcp;
                }
                // Turning DHCP on disables the address fields; don't sit on one.
                if self.is_disabled(self.field()) {
                    self.step_focus(1);
                }
            }
            _ => {
                if let Some(input) = self.input_mut(field) {
                    match key.code {
                        KeyCode::Char(c) => input.insert(c),
                        KeyCode::Backspace => input.backspace(),
                        KeyCode::Delete => input.delete(),
                        KeyCode::Left => input.left(),
                        KeyCode::Right => input.right(),
                        KeyCode::Home => input.home(),
                        KeyCode::End => input.end(),
                        _ => {}
                    }
                }
            }
        }
        FormAction::Stay
    }

    fn step_focus(&mut self, delta: isize) {
        let len = Field::ORDER.len() as isize;
        let mut next = self.focus as isize;
        // Skip over fields DHCP has disabled.
        for _ in 0..len {
            next = (next + delta).rem_euclid(len);
            if !self.is_disabled(Field::ORDER[next as usize]) {
                break;
            }
        }
        self.focus = next as usize;
    }

    /// Validate and convert into something the client can send.
    pub fn to_settings(&self) -> Result<(Credentials, IpSettings), String> {
        let description = self.description.value.trim().to_string();
        if !valid_description(&description) {
            return Err(
                "Description must be 1-32 letters, digits or hyphens, and start and end \
                 with a letter or digit"
                    .to_string(),
            );
        }
        if self.username.value.is_empty() {
            return Err("Username is required to apply changes".to_string());
        }

        let parse = |input: &TextInput, what: &str| -> Result<Ipv4Addr, String> {
            input
                .value
                .trim()
                .parse::<Ipv4Addr>()
                .map_err(|_| format!("{what} is not a valid IPv4 address"))
        };

        let settings = if self.dhcp {
            IpSettings {
                description,
                dhcp: true,
                ip: Ipv4Addr::UNSPECIFIED,
                netmask: Ipv4Addr::UNSPECIFIED,
                gateway: Ipv4Addr::UNSPECIFIED,
            }
        } else {
            IpSettings {
                description,
                dhcp: false,
                ip: parse(&self.ip, "IP address")?,
                netmask: parse(&self.netmask, "Subnet mask")?,
                gateway: parse(&self.gateway, "Gateway")?,
            }
        };

        Ok((
            Credentials {
                username: self.username.value.clone(),
                password: self.password.value.clone(),
            },
            settings,
        ))
    }
}

/// The utility enforces `^[A-Za-z\d](?:[\dA-Za-z-]*[A-Za-z\d])?$`.
fn valid_description(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() || chars.len() > 32 {
        return false;
    }
    let edge_ok = |c: char| c.is_ascii_alphanumeric();
    if !edge_ok(chars[0]) || !edge_ok(chars[chars.len() - 1]) {
        return false;
    }
    chars.iter().all(|&c| c.is_ascii_alphanumeric() || c == '-')
}

/// What a keypress in the settings form asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    Stay,
    Cancel,
    Submit,
}

pub enum Mode {
    List,
    Settings(SettingsForm),
    /// Applying is irreversible enough to be worth one keypress of friction.
    Confirm(SettingsForm),
    Help,
}

/// Work that blocks on the network. The event loop paints the busy line
/// before running it, so a scan never looks like a freeze.
pub enum Pending {
    Refresh,
    OpenSettings,
    // Boxed: the form dwarfs the other variants, and this enum is stored in
    // App for the lifetime of the program.
    Apply(Box<SettingsForm>),
}

pub struct Status {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// How long a status line stays on screen before it is cleared.
const STATUS_TTL: Duration = Duration::from_secs(6);

impl Status {
    pub fn expired(&self) -> bool {
        self.at.elapsed() > STATUS_TTL
    }
}

pub struct App {
    pub client: Client,
    pub switches: Vec<SwitchInfo>,
    pub selected: usize,
    pub mode: Mode,
    pub status: Option<Status>,
    pub creds: Credentials,
    pub busy: Option<String>,
    pub pending: Option<Pending>,
    pub should_quit: bool,
}

impl App {
    pub fn new(client: Client) -> App {
        App {
            client,
            switches: Vec::new(),
            selected: 0,
            mode: Mode::List,
            status: None,
            creds: Credentials::default(),
            busy: None,
            pending: None,
            should_quit: false,
        }
    }

    /// Queue blocking work and describe it, for the frame drawn just before.
    fn defer(&mut self, what: Pending, note: &str) {
        self.busy = Some(note.to_string());
        self.pending = Some(what);
    }

    /// Run whatever `defer` queued. Called by the event loop after it has had
    /// a chance to paint.
    pub fn run_pending(&mut self) {
        let Some(work) = self.pending.take() else { return };
        match work {
            Pending::Refresh => self.refresh(),
            Pending::OpenSettings => self.open_settings(),
            Pending::Apply(form) => self.apply(*form),
        }
        self.busy = None;
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: false, at: Instant::now() });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: true, at: Instant::now() });
    }

    /// Drop a status line once it has had its time on screen.
    pub fn tick(&mut self) {
        if self.status.as_ref().is_some_and(Status::expired) {
            self.status = None;
        }
    }

    pub fn selected_switch(&self) -> Option<&SwitchInfo> {
        self.switches.get(self.selected)
    }

    /// Queue the startup scan.
    pub fn queue_initial_scan(&mut self) {
        self.defer(Pending::Refresh, "Scanning for switches...");
    }

    pub fn refresh(&mut self) {
        let previous = self.selected_switch().map(|s| s.mac);
        match self.client.discover(DISCOVERY_WINDOW) {
            Ok(found) => {
                let count = found.len();
                self.switches = found;
                // Keep the cursor on the same switch across a refresh.
                self.selected = previous
                    .and_then(|mac| self.switches.iter().position(|s| s.mac == mac))
                    .unwrap_or(self.selected)
                    .min(self.switches.len().saturating_sub(1));
                if count == 0 {
                    self.error("No switches answered. Check the interface with --interface.");
                } else {
                    self.info(format!("Found {count} switch{}", if count == 1 { "" } else { "es" }));
                }
            }
            Err(e) => self.error(format!("Discovery failed: {e}")),
        }
    }

    fn open_web_ui(&mut self) {
        let Some(switch) = self.selected_switch() else { return };
        if !switch.reachable_from(self.client.interface()) {
            let ip = switch.ip;
            self.error(format!(
                "{ip} is not on this subnet -- set a reachable address first"
            ));
            return;
        }
        let url = switch.web_url();
        match std::process::Command::new("xdg-open")
            .arg(&url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => self.info(format!("Opened {url}")),
            Err(e) => self.error(format!("Could not open browser: {e}")),
        }
    }

    fn open_settings(&mut self) {
        let Some(switch) = self.selected_switch().cloned() else { return };
        // Re-read so the form shows current values rather than cached ones.
        match self.client.fetch(switch.mac, REPLY_WINDOW) {
            Ok(Some(fresh)) => {
                self.mode = Mode::Settings(SettingsForm::from_switch(&fresh, &self.creds));
                if let Some(slot) = self.switches.get_mut(self.selected) {
                    *slot = fresh;
                }
            }
            Ok(None) => {
                self.error("Switch did not respond; showing last known values");
                self.mode = Mode::Settings(SettingsForm::from_switch(&switch, &self.creds));
            }
            Err(e) => self.error(format!("Read failed: {e}")),
        }
    }

    fn apply(&mut self, form: SettingsForm) {
        let (creds, settings) = match form.to_settings() {
            Ok(v) => v,
            Err(message) => {
                self.error(message);
                self.mode = Mode::Settings(form);
                return;
            }
        };
        // Remember the credentials so the next switch pre-fills them.
        self.creds = creds.clone();

        let result = self
            .client
            .apply_settings(form.mac, &creds, &settings, REPLY_WINDOW);

        match result {
            Err(e) => {
                self.error(format!("Send failed: {e}"));
                self.mode = Mode::Settings(form);
            }
            Ok(Err(message)) => {
                self.error(format!("Apply failed: {message}"));
                self.mode = Mode::Settings(form);
            }
            Ok(Ok(())) => {
                // The vendor utility always follows an apply with a save; its
                // "save config" checkbox is present but hidden and hardwired on.
                let mut note = String::from("Settings applied");
                match self.client.save_config(form.mac, &creds, REPLY_WINDOW) {
                    Ok(Ok(())) => note.push_str(" and saved to flash"),
                    Ok(Err(m)) => note.push_str(&format!(", but saving to flash failed: {m}")),
                    Err(e) => note.push_str(&format!(", but saving to flash failed: {e}")),
                }
                self.mode = Mode::List;
                self.busy = Some("Rescanning...".into());
                self.refresh();
                self.info(note);
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::List => self.on_key_list(key),
            Mode::Help => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => self.mode = Mode::List,
                _ => self.mode = Mode::Help,
            },
            Mode::Settings(form) => self.on_key_settings(key, form),
            Mode::Confirm(form) => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    // Keep the dialog on screen, with a busy line, while the
                    // write is in flight.
                    self.defer(Pending::Apply(Box::new(form.clone())), "Applying...");
                    self.mode = Mode::Confirm(form);
                }
                _ => {
                    self.mode = Mode::Settings(form);
                    self.info("Cancelled");
                }
            },
        }
    }

    fn on_key_list(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('r') => self.defer(Pending::Refresh, "Scanning for switches..."),
            KeyCode::Char('w') => self.open_web_ui(),
            KeyCode::Enter | KeyCode::Char('s') => {
                if self.selected_switch().is_some() {
                    self.defer(Pending::OpenSettings, "Reading settings...");
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.switches.is_empty() {
                    self.selected = (self.selected + 1) % self.switches.len();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.switches.is_empty() {
                    self.selected =
                        (self.selected + self.switches.len() - 1) % self.switches.len();
                }
            }
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.switches.len().saturating_sub(1)
            }
            _ => {}
        }
    }

    fn on_key_settings(&mut self, key: KeyEvent, mut form: SettingsForm) {
        self.mode = match form.on_key(key) {
            FormAction::Cancel => Mode::List,
            FormAction::Submit => Mode::Confirm(form),
            FormAction::Stay => Mode::Settings(form),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_rules_match_the_utility() {
        assert!(valid_description("TL-SG108PE"));
        assert!(valid_description("switch1"));
        assert!(valid_description("a"));
        assert!(!valid_description(""));
        assert!(!valid_description("-leading"));
        assert!(!valid_description("trailing-"));
        assert!(!valid_description("has space"));
        assert!(!valid_description("under_score"));
        assert!(!valid_description(&"x".repeat(33)));
    }

    #[test]
    fn text_input_edits_at_the_cursor() {
        let mut t = TextInput::new("abc", 8);
        t.left();
        t.insert('X');
        assert_eq!(t.value, "abXc");
        t.backspace();
        assert_eq!(t.value, "abc");
        t.home();
        t.delete();
        assert_eq!(t.value, "bc");
    }

    #[test]
    fn text_input_respects_max_len() {
        let mut t = TextInput::new("", 2);
        for c in "abcd".chars() {
            t.insert(c);
        }
        assert_eq!(t.value, "ab");
    }
}
