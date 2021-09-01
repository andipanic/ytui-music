use crate::ui;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use std::collections::VecDeque;
use std::{
    convert::TryFrom,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

pub const MIDDLE_MUSIC_INDEX: usize = 0;
pub const MIDDLE_PLAYLIST_INDEX: usize = 1;
pub const MIDDLE_ARTIST_INDEX: usize = 2;
const SEARCH_SH_KEY: char = '/';
const HELP_SH_KEY: char = '?';
const NEXT_SH_KEY: char = 'n';
const PREV_SH_KEY: char = 'p';
const QUIT_SH_KEY: char = 'q';
const SEEK_F_KEY: char = '>';
const SEEK_B_KEY: char = '<';
const TOGGLE_PAUSE_KEY: char = ' ';
const REFRESH_RATE: u64 = 950;

enum HeadTo {
    Initial,
    Next,
    Prev,
}

fn advance_index(current: usize, limit: usize, direction: HeadTo) -> usize {
    match direction {
        HeadTo::Next => (current + 1) % limit,
        HeadTo::Prev => current.checked_sub(1).unwrap_or(limit - 1) % limit,
        HeadTo::Initial => current,
    }
}

fn advance_list<T>(list: &mut VecDeque<T>, direction: HeadTo) -> bool {
    if list.is_empty() {
        return false;
    }
    match direction {
        HeadTo::Next => list.rotate_left(1),
        HeadTo::Prev => list.rotate_right(1),
        HeadTo::Initial => return false,
    }
    true
}
macro_rules! drop_and_call {
    ($state: expr, $callback: expr) => {{
        std::mem::drop($state);
        $callback()
    }};
    ($state: expr, $callback: expr, $($args: expr)*) => {{
        std::mem::drop($state);
        $callback( $($args)* )
    }};
}

#[inline]
fn get_page(current: &Option<usize>, direction: HeadTo) -> usize {
    let page = match current {
        None => 0,
        Some(prev) => match direction {
            HeadTo::Initial => 0,
            HeadTo::Next => prev + 1,
            HeadTo::Prev => prev.checked_sub(1).unwrap_or_default(),
        },
    };
    page as usize
}

macro_rules! fill_search {
    ("music", $state_original: expr, $notifier: expr, $direction: expr) => {
        fill_search!("@internal-core", $state_original, $notifier, $direction, MIDDLE_MUSIC_INDEX);
        $state_original.lock().unwrap().filled_source.0 = ui::MusicbarSource::Search;
    };
    ("playlist", $state_original: expr, $notifier: expr, $direction: expr) => {
        fill_search!("@internal-core", $state_original, $notifier, $direction, MIDDLE_PLAYLIST_INDEX);
        $state_original.lock().unwrap().filled_source.0 = ui::MusicbarSource::Search;
    };
    ("artist", $state_original: expr, $notifier: expr, $direction: expr) => {
        fill_search!("@internal-core", $state_original, $notifier, $direction, MIDDLE_ARTIST_INDEX);
        $state_original.lock().unwrap().filled_source.0 = ui::MusicbarSource::Search;
    };
    ("all", $state_original: expr, $notifier: expr, $direction: expr) => {
        fill_search!("@internal-core", $state_original, $notifier, $direction, MIDDLE_MUSIC_INDEX, MIDDLE_PLAYLIST_INDEX, MIDDLE_ARTIST_INDEX);
        {
            let mut state = $state_original.lock().unwrap();
            state.filled_source.0 = ui::MusicbarSource::Search;
            state.filled_source.1 = ui::PlaylistbarSource::Search;
            state.filled_source.2 = ui::ArtistbarSource::Search;
        }
    };

    ("@internal-core", $state_original: expr, $notifier: expr, $direction: expr, $($win_index: expr),+  ) => {{
        let mut state = $state_original.lock().unwrap();
        let mut to_search = [None; 3];
        #[allow(unused_mut)]
        let mut page;
        $(
            page = get_page(&state.fetched_page[$win_index], $direction);
            to_search[$win_index] = Some(page);
        )+
        state.to_fetch = ui::FillFetch::Search(state.search.1.clone(), to_search);
        state.help = "Searching..";
        $notifier.notify_all();
    }};
}

pub fn event_sender(state_original: &mut Arc<Mutex<ui::State>>, notifier: &mut Arc<Condvar>) {
    let advance_sidebar = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        let current = state.sidebar.selected().unwrap_or_default();
        state.sidebar.select(Some(advance_index(
            current,
            ui::utils::SIDEBAR_LIST_COUNT,
            direction,
        )));
        notifier.notify_all();
    };
    let advance_music_list = |move_down: HeadTo| {
        if advance_list(&mut state_original.lock().unwrap().musicbar, move_down) {
            notifier.notify_all();
        }
    };
    let advance_artist_list = |move_down: HeadTo| {
        if advance_list(&mut state_original.lock().unwrap().artistbar, move_down) {
            notifier.notify_all();
        }
    };
    let advance_playlist_list = |move_down: HeadTo| {
        if advance_list(&mut state_original.lock().unwrap().playlistbar, move_down) {
            notifier.notify_all();
        }
    };
    let quit = || {
        // setting active window to None is to quit
        state_original.lock().unwrap().active = ui::Window::None;
        notifier.notify_all();
    };
    let moveto_next_window = || {
        let mut state = state_original.lock().unwrap();
        state.active = state.active.next();
        notifier.notify_all();
    };
    let moveto_prev_window = || {
        let mut state = state_original.lock().unwrap();
        state.active = state.active.prev();
        notifier.notify_all();
    };
    let handle_esc = || {
        let mut state = state_original.lock().unwrap();
        if state.active == ui::Window::Searchbar {
            state.search.0.clear();
            drop_and_call!(state, moveto_next_window);
        }
    };
    let handle_backspace = || {
        let mut state = state_original.lock().unwrap();
        match state.active {
            ui::Window::Searchbar => {
                state.search.0.pop();
                notifier.notify_all();
            }
            _ => drop_and_call!(state, moveto_prev_window),
        }
    };
    let handle_search_input = |ch| {
        state_original.lock().unwrap().search.0.push(ch);
        notifier.notify_all();
    };
    let activate_search = || {
        let mut state = state_original.lock().unwrap();
        state.active = ui::Window::Searchbar;
        // Mark search option to be real active
        // this bring state to same state weather
        // activated from shortcut key or from sidebar
        notifier.notify_all();
    };
    let show_help = || {
        todo!();
    };
    let handle_up = || {
        let state = state_original.lock().unwrap();
        match state.active {
            ui::Window::Sidebar => drop_and_call!(state, advance_sidebar, HeadTo::Prev),
            ui::Window::Musicbar => drop_and_call!(state, advance_music_list, HeadTo::Prev),
            ui::Window::Playlistbar => drop_and_call!(state, advance_playlist_list, HeadTo::Prev),
            ui::Window::Artistbar => drop_and_call!(state, advance_artist_list, HeadTo::Prev),
            _ => drop_and_call!(state, moveto_prev_window),
        }
    };
    let handle_down = || {
        let state = state_original.lock().unwrap();
        match state.active {
            ui::Window::Sidebar => drop_and_call!(state, advance_sidebar, HeadTo::Next),
            ui::Window::Musicbar => drop_and_call!(state, advance_music_list, HeadTo::Next),
            ui::Window::Playlistbar => drop_and_call!(state, advance_playlist_list, HeadTo::Next),
            ui::Window::Artistbar => drop_and_call!(state, advance_artist_list, HeadTo::Next),
            _ => drop_and_call!(state, moveto_next_window),
        }
    };

    let fill_search_music = |direction: HeadTo| {
        fill_search!("music", state_original, notifier, direction);
    };
    let fill_search_playlist = |direction: HeadTo| {
        fill_search!("playlist", state_original, notifier, direction);
    };
    let fill_search_artist = |direction: HeadTo| {
        fill_search!("artist", state_original, notifier, direction);
    };

    let fill_trending_music = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        let page = get_page(&state.fetched_page[MIDDLE_MUSIC_INDEX], direction);
        state.to_fetch = ui::FillFetch::Trending(page);
        state.help = "Fetching..";
        notifier.notify_all();
    };
    let fill_community_music = |_direction: HeadTo| {
        //   fill!("community music", direction, state_original, notifier);
    };
    let fill_recents_music = |_direction: HeadTo| {
        // fill!("recents music", direction, state_original, notifier);
    };
    let fill_favourates_music = |_direction: HeadTo| {
        // fill!("favourates music", direction, state_original, notifier);
    };
    let fill_following_artist = |_direction: HeadTo| {
        // fill!("following artist", direction, state_original, notifier);
    };
    let fill_music_from_playlist = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        if let ui::MusicbarSource::Playlist(..) = &state.filled_source.0 {
            let page = get_page(&state.fetched_page[MIDDLE_MUSIC_INDEX], direction);
            state.fetched_page[MIDDLE_MUSIC_INDEX] = Some(page);
            state.to_fetch = ui::FillFetch::Playlist;
            notifier.notify_all();
        }
    };
    let fill_playlist_from_artist = |direction: HeadTo| {
        let mut state = state_original.lock().unwrap();
        if let ui::PlaylistbarSource::Artist(..) = &state.filled_source.1 {
            let page = get_page(&state.fetched_page[MIDDLE_PLAYLIST_INDEX], direction);
            state.fetched_page[MIDDLE_PLAYLIST_INDEX] = Some(page);
            notifier.notify_all();
        }
    };
    let handle_play_advance = |direction: HeadTo| {
        advance_music_list(direction);
        state_original
            .lock()
            .unwrap()
            .play_first_of_musicbar(notifier);
    };
    let handle_page_nav = |direction: HeadTo| {
        let state = state_original.lock().unwrap();
        match state.active {
            ui::Window::Musicbar => match &state.filled_source.0 {
                ui::MusicbarSource::Trending => {
                    drop_and_call!(state, fill_trending_music, direction);
                }
                ui::MusicbarSource::YoutubeCommunity => {
                    drop_and_call!(state, fill_community_music, direction);
                }
                ui::MusicbarSource::RecentlyPlayed => {
                    drop_and_call!(state, fill_recents_music, direction);
                }
                ui::MusicbarSource::Favourates => {
                    drop_and_call!(state, fill_favourates_music, direction);
                }
                ui::MusicbarSource::Search => {
                    drop_and_call!(state, fill_search_music, direction);
                }
                ui::MusicbarSource::Playlist(_) => {
                    drop_and_call!(state, fill_music_from_playlist, direction);
                }
                ui::MusicbarSource::Artist(_) => {}
            },
            ui::Window::Playlistbar => match state.filled_source.1 {
                ui::PlaylistbarSource::Search => {
                    drop_and_call!(state, fill_search_playlist, direction);
                }
                ui::PlaylistbarSource::Artist(_) => {
                    todo!();
                }
                ui::PlaylistbarSource::Favourates | ui::PlaylistbarSource::RecentlyPlayed => {}
            },
            ui::Window::Artistbar => match state.filled_source.2 {
                ui::ArtistbarSource::Followings => {
                    drop_and_call!(state, fill_following_artist, direction);
                }
                ui::ArtistbarSource::Search => {
                    drop_and_call!(state, fill_search_artist, direction);
                }
                ui::ArtistbarSource::RecentlyPlayed => {}
            },
            _ => {}
        }
    };
    let handle_enter = || {
        let mut state = state_original.lock().unwrap();
        let active_window = state.active.clone();
        match active_window {
            ui::Window::Sidebar => {
                let side_select =
                    ui::SidebarOption::try_from(state.sidebar.selected().unwrap()).unwrap();

                match side_select {
                    ui::SidebarOption::Trending => {
                        state.fetched_page[MIDDLE_MUSIC_INDEX] = None;
                        state.filled_source.0 = ui::MusicbarSource::Trending;
                        state.musicbar.clear();
                        drop_and_call!(state, fill_trending_music, HeadTo::Initial);
                    }
                    ui::SidebarOption::YoutubeCommunity => {
                        state.filled_source.0 = ui::MusicbarSource::YoutubeCommunity;
                        state.musicbar.clear();
                        drop_and_call!(state, fill_community_music, HeadTo::Initial);
                    }
                    ui::SidebarOption::Favourates => {
                        // TODOD: also fill favourates artist and playlist
                        state.filled_source.0 = ui::MusicbarSource::Favourates;
                        state.filled_source.1 = ui::PlaylistbarSource::Favourates;
                        state.filled_source.2 = ui::ArtistbarSource::Followings;
                        state.musicbar.clear();
                        state.playlistbar.clear();
                        state.artistbar.clear();
                        drop_and_call!(state, fill_favourates_music, HeadTo::Initial);
                    }
                    ui::SidebarOption::RecentlyPlayed => {
                        // TODO: also fill recently played playlist and artist
                        state.filled_source.0 = ui::MusicbarSource::RecentlyPlayed;
                        state.filled_source.1 = ui::PlaylistbarSource::RecentlyPlayed;
                        state.filled_source.2 = ui::ArtistbarSource::RecentlyPlayed;
                        state.musicbar.clear();
                        state.playlistbar.clear();
                        state.artistbar.clear();
                        drop_and_call!(state, fill_recents_music, HeadTo::Initial);
                    }
                    ui::SidebarOption::Search => drop_and_call!(state, activate_search),
                    ui::SidebarOption::None => {}
                }
            }
            ui::Window::Musicbar => {
                state.play_first_of_musicbar(&notifier);
            }
            ui::Window::Searchbar => {
                state.search.1 = state.search.0.trim().to_string();
                state.search.1.shrink_to_fit();
                state.fetched_page = [None; 3];
                state.musicbar.clear();
                state.playlistbar.clear();
                state.artistbar.clear();
                std::mem::drop(state);
                fill_search!("all", state_original, notifier, HeadTo::Initial);
            }
            ui::Window::Playlistbar => {
                if let Some(playlist) = state.playlistbar.front() {
                    // Fill the music bar with items in this playlist
                    state.filled_source.0 = ui::MusicbarSource::Playlist(playlist.id.clone());
                    state.fetched_page[MIDDLE_MUSIC_INDEX] = None;
                    state.musicbar.clear();
                    drop_and_call!(state, fill_music_from_playlist, HeadTo::Initial);
                }
            }
            ui::Window::Artistbar => {
                if let Some(artist) = state.artistbar.front() {
                    // fill playlistbar & artistbar with items contained in this artist channel
                    let artist_id = artist.id.clone();
                    state.filled_source.0 = ui::MusicbarSource::Artist(artist_id.clone());
                    state.filled_source.1 = ui::PlaylistbarSource::Artist(artist_id);
                    state.fetched_page[MIDDLE_MUSIC_INDEX] = None;
                    state.fetched_page[MIDDLE_PLAYLIST_INDEX] = None;
                    state.musicbar.clear();
                    state.playlistbar.clear();
                    std::mem::drop(state);
                    fill_playlist_from_artist(HeadTo::Initial);
                }
            }
            ui::Window::None | ui::Window::Helpbar => {}
        }
    };

    'listener_loop: loop {
        if event::poll(Duration::from_millis(REFRESH_RATE)).unwrap() {
            match event::read().unwrap() {
                Event::Key(key) => match key.code {
                    KeyCode::Down | KeyCode::PageDown => {
                        handle_down();
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        handle_up();
                    }
                    KeyCode::Right | KeyCode::Tab => {
                        moveto_next_window();
                    }
                    KeyCode::Left | KeyCode::BackTab => {
                        moveto_prev_window();
                    }
                    KeyCode::Esc => {
                        handle_esc();
                    }
                    KeyCode::Enter => {
                        handle_enter();
                    }
                    KeyCode::Backspace | KeyCode::Delete => {
                        handle_backspace();
                    }
                    KeyCode::Char(ch) => {
                        /* If searchbar is active register every char key as input term */
                        if state_original.lock().unwrap().active == ui::Window::Searchbar {
                            handle_search_input(ch);
                        }
                        /* Handle single character key shortcut as it is not in input */
                        else if ch == SEARCH_SH_KEY {
                            activate_search();
                        } else if ch == HELP_SH_KEY {
                            show_help();
                        } else if ch == QUIT_SH_KEY {
                            quit();
                            break 'listener_loop;
                        } else if ch == NEXT_SH_KEY {
                            if key.modifiers.contains(KeyModifiers::CONTROL) {
                                handle_play_advance(HeadTo::Next);
                            } else {
                                handle_page_nav(HeadTo::Next);
                            }
                        } else if ch == PREV_SH_KEY {
                            if key.modifiers.contains(KeyModifiers::CONTROL) {
                                handle_play_advance(HeadTo::Prev);
                            } else {
                                handle_page_nav(HeadTo::Prev);
                            }
                        } else if ch == SEEK_F_KEY {
                        } else if ch == SEEK_B_KEY {
                        } else if ch == TOGGLE_PAUSE_KEY {
                            state_original.lock().unwrap().toggle_pause(notifier);
                        }
                    }
                    _ => {}
                },
                Event::Resize(..) => {
                    // just update the layout
                    notifier.notify_all();
                }
                Event::Mouse(..) => {}
            }
        } else {
            notifier.notify_all();
        }
    }
}
