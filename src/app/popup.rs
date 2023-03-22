use std::{error::Error, mem};

use crossterm::event::KeyCode;
use tui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::events::Event;

use super::MyBackend;

pub enum Message {
    Nothing,
    Quit,
    Commit(String),
}

/// A popup box that asks for user input
#[derive(Debug, Default)]
pub struct Popup {
    title: String,
    input: String,
}

impl Popup {
    pub fn new(title: &str) -> Self {
        Self {
            title: format!(" {} ", title),
            input: String::new(),
        }
    }

    pub fn handle_event(&mut self, event: Event) -> Result<Message, Box<dyn Error>> {
        use Event::*;
        use KeyCode::*;
        if let Terminal(crossterm::event::Event::Key(event)) = event {
            match event.code {
                Char(c) => self.input.push(c),
                Backspace => {
                    self.input.pop();
                }
                Esc => {
                    self.input.clear();
                    return Ok(Message::Quit);
                }
                Enter => {
                    let input = mem::take(&mut self.input);
                    return Ok(Message::Commit(input));
                }
                _ => {},
            }
        }
        Ok(Message::Nothing)
    }

    pub fn render(&mut self, frame: &mut Frame<'_, MyBackend>) {
        let size = frame.size();
        let chunk = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(48),
                Constraint::Length(5),
                Constraint::Percentage(48),
            ])
            .split(size)[1];
        let chunk = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 4),
                Constraint::Ratio(2, 4),
                Constraint::Ratio(1, 4),
            ])
            .split(chunk)[1];

        let block = Block::default()
            .title(self.title.as_str())
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::LightBlue));

        let paragraph = Paragraph::new(format!("\n{}", self.input))
            .block(block)
            .alignment(Alignment::Center);

        frame.render_widget(Clear, chunk);
        frame.render_widget(paragraph, chunk);
    }
}
