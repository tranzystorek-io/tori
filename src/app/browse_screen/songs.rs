use std::borrow::Cow;

use std::{error::Error, path::Path};

use crate::events::Event;
use crate::m3u;
use crate::util::ClickInfo;
use crate::{
    app::{component::Component, filtered_list::FilteredList, App, Mode, MyBackend},
    config::Config,
};

use clipboard::{ClipboardContext, ClipboardProvider};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEventKind};
use tui::layout::Rect;
use tui::{
    layout::{self, Constraint},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Row, Table, TableState},
    Frame,
};

struct MyTableState {
    offset: usize,
    _selected: Option<usize>,
}

fn horrible_hack_to_get_offset(state: &TableState) -> usize {
    // SAFETY: there are some tests that try to check if ListState and MyListState have the same
    // layout, but that's it :)
    unsafe { std::mem::transmute::<&TableState, &MyTableState>(state).offset }
}

/////////////////////////////////
//        SortingMethod        //
/////////////////////////////////
#[derive(Debug, Default, Clone, Copy)]
enum SortingMethod {
    #[default]
    /// identity permutation
    Index,
    Title,
    Duration,
}

impl SortingMethod {
    pub fn next(&self) -> Self {
        use SortingMethod::*;
        match self {
            Index => Title,
            Title => Duration,
            Duration => Index,
        }
    }
}

fn compare_songs(
    i: usize,
    j: usize,
    songs: &[m3u::Song],
    method: SortingMethod,
) -> std::cmp::Ordering {
    match method {
        SortingMethod::Index => i.cmp(&j),
        SortingMethod::Title => songs[i].title.cmp(&songs[j].title),
        SortingMethod::Duration => songs[i].duration.cmp(&songs[j].duration),
    }
}

#[derive(Debug, Default)]
pub struct SongsPane<'t> {
    title: Cow<'t, str>,
    songs: Vec<m3u::Song>,
    shown: FilteredList<TableState>,
    sorting_method: SortingMethod,
    filter: String,
    last_click: Option<ClickInfo>,
}

impl<'t> SongsPane<'t> {
    pub fn new() -> Self {
        Self {
            title: " songs ".into(),
            ..Default::default()
        }
    }

    pub fn state(&self) -> TableState {
        self.shown.state.clone()
    }
    pub fn set_state(&mut self, state: TableState) {
        self.shown.state = state;
    }

    pub fn update_from_playlist_pane(
        &mut self,
        playlists: &super::playlists::PlaylistsPane,
    ) -> Result<(), Box<dyn Error>> {
        match playlists.selected_item() {
            Some(playlist) => self.update_from_playlist_named(playlist),
            None => {
                *self = SongsPane::new();
                Ok(())
            }
        }
    }

    pub fn update_from_playlist_named(&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        self.update_from_playlist(Config::playlist_path(name))
    }

    pub fn update_from_playlist(&mut self, path: impl AsRef<Path>) -> Result<(), Box<dyn Error>> {
        // TODO: maybe return Result?
        let file = std::fs::File::open(&path)
            .unwrap_or_else(|_| panic!("Couldn't open playlist file {}", path.as_ref().display()));

        let title = Cow::Owned(
            path.as_ref()
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_string(),
        );

        let songs = m3u::Parser::from_reader(file).all_songs()?;
        let state = self.state();

        // Update stuff
        self.title = title;
        self.songs = songs;
        self.filter.clear();
        self.refresh_shown();

        // Try to reuse previous state
        if matches!(state.selected(), Some(i) if i < self.songs.len()) {
            self.set_state(state);
        } else if self.shown.items.is_empty() {
            self.select_index(None);
        } else {
            self.select_index(Some(0));
        }

        Ok(())
    }

    fn refresh_shown(&mut self) {
        let pred = |s: &m3u::Song| {
            self.filter.is_empty()
                || s.title
                    .to_lowercase()
                    .contains(&self.filter[1..].trim_end_matches('\n').to_lowercase())
                || s.path
                    .to_lowercase()
                    .contains(&self.filter[1..].trim_end_matches('\n').to_lowercase())
        };
        let comparison = |i, j| compare_songs(i, j, &self.songs, self.sorting_method);
        self.shown.filter(&self.songs, pred, comparison);
    }

    fn next_sorting_method(&mut self) {
        self.sorting_method = self.sorting_method.next();
    }

    fn handle_terminal_event(
        &mut self,
        app: &mut App,
        event: crossterm::event::Event,
    ) -> Result<(), Box<dyn Error>> {
        use KeyCode::*;

        match event {
            crossterm::event::Event::Key(event) => {
                if self.mode() == Mode::Insert && self.handle_filter_key_event(event)? {
                    self.refresh_shown();
                    return Ok(());
                }

                match event.code {
                    Enter => self.play_selected(app)?,
                    Esc => {
                        self.filter.clear();
                        self.refresh_shown();
                    }
                    // Go to the bottom, also like in vim
                    Char('G') => {
                        if !self.shown.items.is_empty() {
                            self.shown.state.select(Some(self.shown.items.len() - 1));
                        }
                    }
                    Up => self.select_prev(),
                    Down => self.select_next(),
                    Char('/') => self.filter = "/".into(),
                    _ => {}
                }
            }
            crossterm::event::Event::Mouse(event) => match event.kind {
                MouseEventKind::ScrollUp => self.select_prev(),
                MouseEventKind::ScrollDown => self.select_next(),
                MouseEventKind::Down(MouseButton::Left) => {
                    let frame_size = app.frame_size();
                    self.click(app, frame_size, event.row)?;
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn handle_command(
        &mut self,
        app: &mut App,
        cmd: crate::command::Command,
    ) -> Result<(), Box<dyn Error>> {
        use crate::command::Command::*;

        match cmd {
            SelectNext => self.select_next(),
            SelectPrev => self.select_prev(),
            QueueSong => {
                if let Some(song) = self.selected_item() {
                    app.mpv.playlist_load_files(&[(
                        &song.path,
                        libmpv::FileState::AppendPlay,
                        None,
                    )])?;
                }
            }
            QueueShown => {
                let entries: Vec<_> = self
                    .shown
                    .items
                    .iter()
                    .map(|&i| {
                        (
                            self.songs[i].path.as_str(),
                            libmpv::FileState::AppendPlay,
                            None::<&str>,
                        )
                    })
                    .collect();
                app.mpv.playlist_load_files(&entries)?;
            }
            Shuffle => {
                app.mpv.command("playlist-shuffle", &[])?;
            }
            OpenInBrowser => {
                if let Some(song) = self.selected_item() {
                    // TODO: reconsider if I really need a library to write this one line
                    webbrowser::open(&song.path)?;
                }
            }
            CopyUrl => {
                if let Some(song) = self.selected_item() {
                    let mut ctx: ClipboardContext = ClipboardProvider::new()?;
                    ctx.set_contents(song.path.clone())?;
                    app.notify_info(format!("Copied {} to the clipboard", song.path));
                }
            }
            CopyTitle => {
                if let Some(song) = self.selected_item() {
                    let mut ctx: ClipboardContext = ClipboardProvider::new()?;
                    ctx.set_contents(song.title.clone())?;
                    app.notify_info(format!("Copied {} to the clipboard", song.title));
                }
            }
            SwapSongUp if self.filter.is_empty() => match self.selected_index() {
                Some(i) if i >= 1 => {
                    m3u::playlist_management::swap_song(&self.title, i - 1)?;
                    self.songs.swap(i - 1, i);
                    self.select_prev();
                }
                _ => {}
            },
            SwapSongDown if self.filter.is_empty() => {
                if let Some(i) = self.selected_index() {
                    m3u::playlist_management::swap_song(&self.title, i)?;
                    self.songs.swap(i, i + 1);
                    self.select_next();
                }
            }
            NextSortingMode => {
                self.next_sorting_method();
                self.refresh_shown();
            }
            _ => {}
        }
        Ok(())
    }

    /// Handles a key event when the filter is active.
    pub fn handle_filter_key_event(&mut self, event: KeyEvent) -> Result<bool, Box<dyn Error>> {
        match event.code {
            KeyCode::Char(c) => {
                self.filter.push(c);
                Ok(true)
            }
            KeyCode::Backspace => {
                self.filter.pop();
                Ok(true)
            }
            KeyCode::Esc => {
                self.filter.clear();
                Ok(true)
            }
            KeyCode::Enter => {
                self.filter.push('\n');
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn play_selected(&self, app: &mut App) -> Result<(), Box<dyn Error>> {
        if let Some(song) = self.selected_item() {
            app.mpv
                .playlist_load_files(&[(&song.path, libmpv::FileState::Replace, None)])?;
        }
        Ok(())
    }

    fn click(&mut self, app: &mut App, frame: Rect, y: u16) -> Result<(), Box<dyn Error>> {
        // Compute clicked item
        let top = frame
            .inner(&layout::Margin {
                vertical: 1,
                horizontal: 1,
            })
            .top();
        let line = y.saturating_sub(top) as usize;
        let index = line + horrible_hack_to_get_offset(&self.shown.state);

        // Update self.last_click with current click
        let click_summary = ClickInfo::update(&mut self.last_click, y);

        // User clicked outside the list
        if index >= self.shown.items.len() {
            return Ok(());
        }

        // Select song
        self.select_index(Some(index));

        // If it's a double click, play this selected song
        if click_summary.double_click {
            self.play_selected(app)?;
        }
        Ok(())
    }

    pub fn select_next(&mut self) {
        self.shown.select_next();
    }

    pub fn select_prev(&mut self) {
        self.shown.select_prev();
    }

    pub fn select_index(&mut self, i: Option<usize>) {
        self.shown.state.select(i);
    }

    pub fn selected_item(&self) -> Option<&m3u::Song> {
        self.shown.selected_item().and_then(|i| self.songs.get(i))
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.shown.selected_item()
    }
}

impl<'t> Component for SongsPane<'t> {
    type RenderState = bool;

    fn mode(&self) -> Mode {
        if self.filter.is_empty() || self.filter.as_bytes().last() == Some(&b'\n') {
            Mode::Normal
        } else {
            Mode::Insert
        }
    }

    fn render(&mut self, frame: &mut Frame<'_, MyBackend>, chunk: layout::Rect, is_focused: bool) {
        let sorting = match self.sorting_method {
            SortingMethod::Index => "",
            SortingMethod::Title => " [↑ Title]",
            SortingMethod::Duration => " [↑ Duration]",
        };

        let title = if !self.filter.is_empty() {
            format!(" {}{} ", self.filter, sorting)
        } else {
            format!(" {}{} ", self.title, sorting)
        };

        let mut block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(BorderType::Plain);

        if is_focused {
            block = block.border_style(Style::default().fg(Color::LightBlue));
        }

        let songlist: Vec<_> = self
            .shown
            .items
            .iter()
            .map(|&i| &self.songs[i])
            .map(|song| {
                Row::new(vec![
                    format!(" {}", song.title),
                    format!(
                        "{}:{:02}",
                        song.duration.as_secs() / 60,
                        song.duration.as_secs() % 60
                    ),
                ])
            })
            .collect();

        let widths = &[Constraint::Length(chunk.width - 11), Constraint::Length(10)];
        let widget = Table::new(songlist)
            .block(block)
            .widths(widths)
            .highlight_style(Style::default().bg(Color::Yellow).fg(Color::Black))
            .highlight_symbol(" ◇");
        frame.render_stateful_widget(widget, chunk, &mut self.shown.state);
    }

    fn handle_event(&mut self, app: &mut App, event: Event) -> Result<(), Box<dyn Error>> {
        use Event::*;

        match event {
            Command(cmd) => self.handle_command(app, cmd)?,
            Terminal(event) => self.handle_terminal_event(app, event)?,
            SongAdded {
                playlist: _,
                song: _,
            } => {
                // scroll to the bottom
                if !self.shown.items.is_empty() {
                    self.shown.state.select(Some(self.shown.items.len() - 1));
                }
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{thread_rng, Rng};
    use std::mem::size_of;
    use tui::buffer::{Buffer, Cell};
    use tui::widgets::StatefulWidget;

    #[test]
    fn test_my_table_state_size() {
        assert_eq!(size_of::<MyTableState>(), size_of::<TableState>());
    }

    #[test]
    fn test_my_table_state_offset() {
        let area = Rect {
            x: 0,
            width: 64,
            y: 0,
            height: 1, // ensures offset == selected
        };
        let mut buf = Buffer {
            area,
            content: vec![Cell::default(); 64],
        };

        let items = 32;
        let mut state = TableState::default();
        for _ in 0..10 {
            let offset = thread_rng().gen_range(0..32);
            state.select(Some(offset));

            // Render table in a really small area so it changes the state.offset
            let table = Table::new(vec![Row::new(vec!["Hello World!"]); items]);
            table.render(area, &mut buf, &mut state);

            // Assert our hack worked
            assert_eq!(horrible_hack_to_get_offset(&state), offset);
        }
    }
}
