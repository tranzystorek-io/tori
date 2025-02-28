use self::centered_list::{CenteredList, CenteredListItem, CenteredListState};
use super::{
    component::{Component, MouseHandler},
    App, Mode,
};
use crate::{command, error::Result, events, player::Player, widgets::Scrollbar};
use std::{thread, time::Duration};
use tui::{
    layout::{Alignment, Rect},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders},
};

mod centered_list;

/// Screen that shows the current mpv playlist. You can press '2' to access it.
#[derive(Debug, Default)]
pub struct PlaylistScreen {
    songs: Vec<String>,
    playing: CenteredListState,
}

impl PlaylistScreen {
    /// See <https://mpv.io/manual/master/#command-interface-playlist>
    pub fn update(&mut self, player: &impl Player) -> Result<&mut Self> {
        let n = player.playlist_count()?;

        self.songs = (0..n)
            .map(|i| player.playlist_track_title(i))
            .collect::<Result<_>>()?;

        self.playing.select(player.playlist_position().ok());

        Ok(self)
    }

    /// Waits a couple of milliseconds, then calls [update](PlaylistScreen::update). It's used
    /// primarily by [select_next](PlaylistScreen::select_next) and
    /// [select_prev](PlaylistScreen::select_prev) because mpv takes a while to update the playlist
    /// properties after changing the selection.
    pub fn update_after_delay(&self, app: &App) {
        let sender = app.channel.sender.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(16));
            sender.send(events::Event::SecondTick).ok();
        });
    }

    fn handle_command(&mut self, app: &mut App, cmd: command::Command) -> Result<()> {
        use command::Command::*;
        match cmd {
            SelectNext | NextSong => self.select_next(app),
            SelectPrev | PrevSong => self.select_prev(app),
            _ => {}
        }
        Ok(())
    }

    fn handle_terminal_event(
        &mut self,
        app: &mut App,
        event: crossterm::event::Event,
    ) -> Result<()> {
        use crossterm::event::{Event, KeyCode};
        if let Event::Key(key_event) = event {
            match key_event.code {
                KeyCode::Up => self.select_prev(app),
                KeyCode::Down => self.select_next(app),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn select_next(&self, app: &mut App) {
        app.player
            .playlist_next()
            .unwrap_or_else(|_| app.notify_err("No next song"));
        self.update_after_delay(app);
    }

    pub fn select_prev(&self, app: &mut App) {
        app.player
            .playlist_previous()
            .unwrap_or_else(|_| app.notify_err("No previous song"));
        self.update_after_delay(app);
    }
}

impl Component for PlaylistScreen {
    type RenderState = ();

    fn mode(&self) -> Mode {
        Mode::Normal
    }

    fn render(&mut self, frame: &mut tui::Frame<'_, super::MyBackend>, chunk: Rect, (): ()) {
        let block = Block::default()
            .title(" Playlist ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::LightRed));

        let items: Vec<_> = self
            .songs
            .iter()
            .map(|x| CenteredListItem::new(x.as_str()))
            .collect();
        let list = CenteredList::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::Red).fg(Color::White))
            .highlight_symbol("›")
            .highlight_symbol_right("‹");

        frame.render_stateful_widget(list, chunk, &mut self.playing);

        if self.songs.len() > chunk.height as usize - 2 {
            if let Some(index) = self.playing.selected() {
                let scrollbar = Scrollbar::new(index as u16, self.songs.len() as u16)
                    .with_style(Style::default().fg(Color::Red));
                frame.render_widget(scrollbar, chunk);
            }
        }
    }

    fn handle_event(&mut self, app: &mut App, event: events::Event) -> Result<()> {
        use events::Event::*;
        match event {
            Command(cmd) => self.handle_command(app, cmd)?,
            Terminal(event) => self.handle_terminal_event(app, event)?,
            SecondTick => {
                self.update(&app.player)?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl MouseHandler for PlaylistScreen {
    fn handle_mouse(
        &mut self,
        _app: &mut App,
        _chunk: Rect,
        _event: crossterm::event::MouseEvent,
    ) -> Result<()> {
        Ok(())
    }
}
