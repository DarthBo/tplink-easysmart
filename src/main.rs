//! A terminal front-end for TP-Link Easy Smart switches (TL-SG1xxE family).
//!
//! Covers the two things the vendor utility is actually needed for -- finding
//! switches on the wire and setting their address -- and hands everything else
//! to the switch's own web interface.

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use tplink_easysmart::app::App;
use tplink_easysmart::proto::client::Client;
use tplink_easysmart::proto::{self, iface};
use tplink_easysmart::ui;

const USAGE: &str = "\
tplink-easysmart -- discover and configure TP-Link Easy Smart switches

USAGE:
    tplink-easysmart [OPTIONS]

OPTIONS:
    -i, --interface <NAME>   Network interface to search on
                             (default: the one carrying the default route)
    -l, --list               List interfaces and exit
    -h, --help               Show this help
";

fn main() -> io::Result<()> {
    let mut interface_name: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-l" | "--list" => {
                for i in iface::list() {
                    println!("{:<10} {:<15} {}", i.name, i.addr, iface::format_mac(&i.mac));
                }
                return Ok(());
            }
            "-i" | "--interface" => match args.next() {
                Some(name) => interface_name = Some(name),
                None => {
                    eprintln!("error: --interface needs a value");
                    std::process::exit(2);
                }
            },
            other => {
                eprintln!("error: unexpected argument '{other}'\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let interface = match &interface_name {
        Some(name) => iface::by_name(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no usable interface named '{name}' (try --list)"),
            )
        })?,
        None => iface::default_interface().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no usable network interface found")
        })?,
    };

    let client = Client::bind(interface).map_err(|e| {
        let hint = if e.kind() == io::ErrorKind::AddrInUse {
            "  (another copy of this tool, or the TP-Link utility, is already running)"
        } else {
            ""
        };
        io::Error::new(
            e.kind(),
            format!("could not bind UDP port {}: {e}{hint}", proto::packet::HOST_PORT),
        )
    })?;

    run(App::new(client))
}

fn run(mut app: App) -> io::Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> io::Result<()> {
    // Paint the empty table with a "scanning" note before the first blocking
    // discovery, so startup doesn't look like a hang.
    app.busy = Some("Scanning for switches...".into());
    terminal.draw(|f| ui::draw(f, app))?;
    app.refresh();
    app.busy = None;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(Duration::from_millis(200))? {
            app.tick();
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            app.on_key(key);
            if app.should_quit {
                return Ok(());
            }
        }
    }
}
