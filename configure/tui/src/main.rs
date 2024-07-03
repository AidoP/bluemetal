use std::io::{stdout, Result};

use ratatui::{backend::CrosstermBackend, crossterm::{event::{self, KeyCode, KeyEvent, KeyEventKind}, terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}, ExecutableCommand}, style::Stylize, widgets::Paragraph, Terminal};

struct RawMode {}
impl RawMode {
    pub fn enable() -> Result<Self> {
        enable_raw_mode()?;
        Ok(RawMode {})
    }
    pub fn disable(self) {}
}
impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn main() -> Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    let raw_mode = RawMode::enable()?;

    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    'main: loop {
        terminal.draw(|frame| {
            let area = frame.size();
            frame.render_widget(Paragraph::new("Hello, World!").white().on_black(), area)
        })?;

        match event::read()? {
            event::Event::Key(KeyEvent {
                code: KeyCode::Char('q'),
                kind: KeyEventKind::Press,
                ..
            }) => break 'main,
            _ => (),
        }
    }

    stdout().execute(LeaveAlternateScreen)?;
    raw_mode.disable();
    Ok(())
}
