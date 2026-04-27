pub(crate) mod deprecated;
pub(crate) mod input;

use crate::actions::cpaste::PasteOverSelection;
use crate::actions::delete::{
    DeleteCharForward, DeleteToEndOfLine, DeleteToFirstCharOfLine, DeleteWordBackward,
    DeleteWordForward,
};
use crate::actions::motion::{
    MoveHalfPageDown, MovePageDown, MovePageUp, MoveToFirstRow, MoveToLastRow,
};
use crate::actions::search::{StartBackwardSearch, StartSearch};
#[cfg(feature = "system-editor")]
use crate::actions::OpenSystemEditor;
use crate::actions::{
    Action, AppendCharToSearch, AppendNewline, Chainable, ChangeToEndOfLine, DeleteChar, Execute,
    FindFirst, FindNext, FindPrevious, InsertChar, InsertNewline, JoinLineWithLineBelow, LineBreak,
    MoveBackward, MoveBigWordBackward, MoveBigWordForward, MoveBigWordForwardToEndOfWord, MoveDown,
    MoveForward, MoveHalfPageUp, MoveToEndOfLine, MoveToFirst, MoveToMatchinBracket,
    MoveToStartOfLine, MoveUp, MoveWordBackward, MoveWordForward, MoveWordForwardToEndOfWord,
    Paste, Redo, RemoveChar, RemoveCharFromSearch, SelectCurrentSearch, StopSearch, SwitchMode,
    Undo,
};
use crate::events::KeyInput;
use crate::vim::{command::VimCommandContext, state::VimCommandState};
use crate::{EditorMode, EditorState};
use crossterm::event::KeyCode;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct KeyEventHandler {
    lookup: Vec<KeyInput>,
    vim: VimCommandState,
    register: HashMap<KeyEventRegister, Action>,
    capture_on_insert: bool,
}

impl Default for KeyEventHandler {
    fn default() -> Self {
        Self::vim_mode()
    }
}

impl KeyEventHandler {
    /// Creates a new `KeyEventHandler`.
    #[must_use]
    pub fn new(register: HashMap<KeyEventRegister, Action>, capture_on_insert: bool) -> Self {
        Self {
            lookup: Vec::new(),
            vim: VimCommandState::default(),
            register,
            capture_on_insert,
        }
    }

    /// Creates a new `KeyEventHandler` with vim keybindings.
    #[must_use]
    pub fn vim_mode() -> Self {
        let register: HashMap<KeyEventRegister, Action> = vim_keybindings();
        Self {
            lookup: Vec::new(),
            vim: VimCommandState::default(),
            register,
            capture_on_insert: false,
        }
    }

    // Creates a new `KeyEventHandler` with emacs keybindings.
    #[must_use]
    pub fn emacs_mode() -> Self {
        let register: HashMap<KeyEventRegister, Action> = emacs_keybindings();
        Self {
            lookup: Vec::new(),
            vim: VimCommandState::default(),
            register,
            capture_on_insert: true,
        }
    }

    /// Insert a new callback to the registry
    pub fn insert<T>(&mut self, key: KeyEventRegister, action: T)
    where
        T: Into<Action>,
    {
        self.register.insert(key, action.into());
    }

    /// Extents the register with the contents of an iterator
    pub fn extend<T, U>(&mut self, iter: T)
    where
        U: Into<Action>,
        T: IntoIterator<Item = (KeyEventRegister, U)>,
    {
        self.register
            .extend(iter.into_iter().map(|(k, v)| (k, v.into())));
    }

    /// Remove a callback from the registry
    pub fn remove(&mut self, key: &KeyEventRegister) {
        self.register.remove(key);
    }

    /// Returns an action for a specific register key, if present.
    /// Returns an action only if there is an exact match. If there
    /// are multiple matches or an inexact match, the specified key
    /// is appended to the lookup vector.
    /// If there is an exact match or if none of the keys in the registry
    /// starts with the current sequence, the lookup sequence is reset.
    #[must_use]
    fn get(&mut self, c: KeyInput, mode: EditorMode) -> Option<Action> {
        self.lookup.push(c);
        let key = KeyEventRegister::new(self.lookup.clone(), mode);

        match self
            .register
            .keys()
            .filter(|k| k.mode == key.mode && k.keys.starts_with(&key.keys))
            .count()
        {
            0 => {
                self.lookup.clear();
                None
            }
            1 => self.register.get(&key).map(|action| {
                self.lookup.clear();
                action.clone()
            }),
            _ => None,
        }
    }
}

#[allow(clippy::too_many_lines)]
fn vim_keybindings() -> HashMap<KeyEventRegister, Action> {
    #[allow(unused_mut)]
    let mut map = HashMap::from([
        // Go into normal mode
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Esc)]),
            SwitchMode(EditorMode::Normal).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::Esc)]),
            SwitchMode(EditorMode::Normal).into(),
        ),
        // Go into insert mode
        (
            KeyEventRegister::n(vec![KeyInput::new('i')]),
            SwitchMode(EditorMode::Insert).into(),
        ),
        // Go into visual mode
        (
            KeyEventRegister::n(vec![KeyInput::new('v')]),
            SwitchMode(EditorMode::Visual).into(),
        ),
        // Goes into search mode and starts of a new search.
        (
            KeyEventRegister::n(vec![KeyInput::new('/')]),
            StartSearch.chain(SwitchMode(EditorMode::Search)).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new('?')]),
            StartBackwardSearch
                .chain(SwitchMode(EditorMode::Search))
                .into(),
        ),
        // Trigger initial search
        (
            KeyEventRegister::s(vec![KeyInput::new(KeyCode::Enter)]),
            FindFirst.chain(SwitchMode(EditorMode::Normal)).into(),
        ),
        // Find next
        (
            KeyEventRegister::n(vec![KeyInput::new('n')]),
            FindNext.into(),
        ),
        // Find previous
        (
            KeyEventRegister::n(vec![KeyInput::shift('N')]),
            FindPrevious.into(),
        ),
        // Clear search
        (
            KeyEventRegister::s(vec![KeyInput::new(KeyCode::Esc)]),
            StopSearch.chain(SwitchMode(EditorMode::Normal)).into(),
        ),
        // Delete last character from search
        (
            KeyEventRegister::s(vec![KeyInput::new(KeyCode::Backspace)]),
            RemoveCharFromSearch.into(),
        ),
        // Go into insert mode and move one char forward
        (
            KeyEventRegister::n(vec![KeyInput::new('a')]),
            SwitchMode(EditorMode::Insert).chain(MoveForward(1)).into(),
        ),
        // Move cursor forward
        (
            KeyEventRegister::n(vec![KeyInput::new('l')]),
            MoveForward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('l')]),
            MoveForward(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::Right)]),
            MoveForward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::Right)]),
            MoveForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Right)]),
            MoveForward(1).into(),
        ),
        // Move cursor backward
        (
            KeyEventRegister::n(vec![KeyInput::new('h')]),
            MoveBackward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('h')]),
            MoveBackward(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::Left)]),
            MoveBackward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::Left)]),
            MoveBackward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Left)]),
            MoveBackward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl(KeyCode::Right)]),
            MoveWordForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl(KeyCode::Left)]),
            MoveWordBackward(1).into(),
        ),
        // Move cursor up
        (
            KeyEventRegister::n(vec![KeyInput::new('k')]),
            MoveUp(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('k')]),
            MoveUp(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::Up)]),
            MoveUp(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::Up)]),
            MoveUp(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Up)]),
            MoveUp(1).into(),
        ),
        // Move cursor down
        (
            KeyEventRegister::n(vec![KeyInput::new('j')]),
            MoveDown(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('j')]),
            MoveDown(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::Down)]),
            MoveDown(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::Down)]),
            MoveDown(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Down)]),
            MoveDown(1).into(),
        ),
        // Move one word forward/backward
        (
            KeyEventRegister::n(vec![KeyInput::new('w')]),
            MoveWordForward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('w')]),
            MoveWordForward(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new('e')]),
            MoveWordForwardToEndOfWord(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('e')]),
            MoveWordForwardToEndOfWord(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new('b')]),
            MoveWordBackward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('b')]),
            MoveWordBackward(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::shift('W')]),
            MoveBigWordForward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::shift('W')]),
            MoveBigWordForward(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::shift('E')]),
            MoveBigWordForwardToEndOfWord(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::shift('E')]),
            MoveBigWordForwardToEndOfWord(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::shift('B')]),
            MoveBigWordBackward(1).into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::shift('B')]),
            MoveBigWordBackward(1).into(),
        ),
        // Move cursor to start/first/last position
        (
            KeyEventRegister::n(vec![KeyInput::new('0')]),
            MoveToStartOfLine().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new('_')]),
            MoveToFirst().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new('$')]),
            MoveToEndOfLine().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('0')]),
            MoveToStartOfLine().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('_')]),
            MoveToFirst().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('$')]),
            MoveToEndOfLine().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::ctrl('d')]),
            MoveHalfPageDown().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::ctrl('d')]),
            MoveHalfPageDown().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::ctrl('u')]),
            MoveHalfPageUp().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::ctrl('u')]),
            MoveHalfPageUp().into(),
        ),
        // Page up/down for full page navigation
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::PageDown)]),
            MovePageDown().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::PageDown)]),
            MovePageDown().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::PageDown)]),
            MovePageDown().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::PageUp)]),
            MovePageUp().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::PageUp)]),
            MovePageUp().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::PageUp)]),
            MovePageUp().into(),
        ),
        // `Home` and `End` go to first/last position in a line
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Home)]),
            MoveToStartOfLine().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::Home)]),
            MoveToStartOfLine().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::Home)]),
            MoveToStartOfLine().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::End)]),
            MoveToEndOfLine().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::End)]),
            MoveToEndOfLine().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new(KeyCode::End)]),
            MoveToEndOfLine().into(),
        ),
        // `Ctrl+u` deltes from cursor to first non-whitespace character in insert mode
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('u')]),
            DeleteToFirstCharOfLine.into(),
        ),
        // Move cursor to start/first/last position and enter insert mode
        (
            KeyEventRegister::n(vec![KeyInput::shift('I')]),
            SwitchMode(EditorMode::Insert).chain(MoveToFirst()).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::shift('A')]),
            SwitchMode(EditorMode::Insert)
                .chain(MoveToEndOfLine())
                .chain(MoveForward(1))
                .into(),
        ),
        // Move cursor to start/last row in the buffer
        (
            KeyEventRegister::n(vec![KeyInput::new('g'), KeyInput::new('g')]),
            MoveToFirstRow().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('g'), KeyInput::new('g')]),
            MoveToFirstRow().into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::shift('G')]),
            MoveToLastRow().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::shift('G')]),
            MoveToLastRow().into(),
        ),
        // Move cursor to the next opening/closing bracket.
        (
            KeyEventRegister::n(vec![KeyInput::new('%')]),
            MoveToMatchinBracket().into(),
        ),
        (
            KeyEventRegister::v(vec![KeyInput::new('%')]),
            MoveToMatchinBracket().into(),
        ),
        // Append/insert new line and switch into insert mode
        (
            KeyEventRegister::n(vec![KeyInput::new('o')]),
            SwitchMode(EditorMode::Insert)
                .chain(AppendNewline(1))
                .into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::shift('O')]),
            SwitchMode(EditorMode::Insert)
                .chain(InsertNewline(1))
                .into(),
        ),
        // Insert a line break
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Enter)]),
            LineBreak(1).into(),
        ),
        // Remove the current character
        (
            KeyEventRegister::n(vec![KeyInput::new('x')]),
            RemoveChar(1).into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::new(KeyCode::Delete)]),
            RemoveChar(1).into(),
        ),
        // Delete the previous character
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Backspace)]),
            DeleteChar(1).into(),
        ),
        // Delete the next character
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Delete)]),
            DeleteCharForward(1).into(),
        ),
        // Delete/change from the cursor to the end of the line
        (
            KeyEventRegister::n(vec![KeyInput::shift('D')]),
            DeleteToEndOfLine.into(),
        ),
        (
            KeyEventRegister::n(vec![KeyInput::shift('C')]),
            ChangeToEndOfLine.into(),
        ),
        // Join the current line with the line below
        (
            KeyEventRegister::n(vec![KeyInput::shift('J')]),
            JoinLineWithLineBelow.into(),
        ),
        // Undo
        (KeyEventRegister::n(vec![KeyInput::new('u')]), Undo.into()),
        // Redo
        (KeyEventRegister::n(vec![KeyInput::ctrl('r')]), Redo.into()),
        // Copy
        // Paste
        (KeyEventRegister::n(vec![KeyInput::new('p')]), Paste.into()),
        (
            KeyEventRegister::v(vec![KeyInput::new('p')]),
            PasteOverSelection
                .chain(SwitchMode(EditorMode::Normal))
                .into(),
        ),
    ]);

    // Open system editor (Ctrl+e in normal mode)
    #[cfg(feature = "system-editor")]
    map.insert(
        KeyEventRegister::n(vec![KeyInput::ctrl('e')]),
        OpenSystemEditor.into(),
    );

    map
}

#[allow(clippy::too_many_lines)]
fn emacs_keybindings() -> HashMap<KeyEventRegister, Action> {
    HashMap::from([
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('s')]),
            StartSearch.chain(SwitchMode(EditorMode::Search)).into(),
        ),
        (
            KeyEventRegister::s(vec![KeyInput::ctrl('s')]),
            FindNext.into(),
        ),
        (
            KeyEventRegister::s(vec![KeyInput::ctrl('r')]),
            FindPrevious.into(),
        ),
        (
            KeyEventRegister::s(vec![KeyInput::new(KeyCode::Enter)]),
            SelectCurrentSearch
                .chain(SwitchMode(EditorMode::Insert))
                .into(),
        ),
        (
            KeyEventRegister::s(vec![KeyInput::ctrl('g')]),
            StopSearch.chain(SwitchMode(EditorMode::Insert)).into(),
        ),
        (
            KeyEventRegister::s(vec![KeyInput::new(KeyCode::Backspace)]),
            RemoveCharFromSearch.into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('f')]),
            MoveForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Right)]),
            MoveForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('b')]),
            MoveBackward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Left)]),
            MoveBackward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('p')]),
            MoveUp(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Up)]),
            MoveUp(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('n')]),
            MoveDown(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Down)]),
            MoveDown(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt('f')]),
            MoveWordForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt('b')]),
            MoveWordBackward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl(KeyCode::Right)]),
            MoveWordForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl(KeyCode::Left)]),
            MoveWordBackward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('v')]),
            MoveHalfPageDown().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt('v')]),
            MoveHalfPageUp().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::PageDown)]),
            MovePageDown().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::PageUp)]),
            MovePageUp().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt('<')]),
            MoveToFirstRow().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt('>')]),
            MoveToLastRow().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('a')]),
            MoveToStartOfLine().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Home)]),
            MoveToStartOfLine().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::End)]),
            MoveToEndOfLine().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('e')]),
            MoveToEndOfLine().into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt('u')]),
            DeleteToFirstCharOfLine.into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('k')]),
            DeleteToEndOfLine.into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('o')]),
            LineBreak(1)
                .chain(MoveUp(1))
                .chain(MoveToEndOfLine())
                .into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Enter)]),
            LineBreak(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('j')]),
            LineBreak(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Backspace)]),
            DeleteChar(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('h')]),
            DeleteChar(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::new(KeyCode::Delete)]),
            DeleteCharForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::ctrl('d')]),
            DeleteCharForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt('d')]),
            DeleteWordForward(1).into(),
        ),
        (
            KeyEventRegister::i(vec![KeyInput::alt(KeyCode::Backspace)]),
            DeleteWordBackward(1).into(),
        ),
        (KeyEventRegister::i(vec![KeyInput::ctrl('u')]), Undo.into()),
        (KeyEventRegister::i(vec![KeyInput::ctrl('r')]), Redo.into()),
        (KeyEventRegister::i(vec![KeyInput::ctrl('y')]), Paste.into()),
        #[cfg(feature = "system-editor")]
        (
            KeyEventRegister::i(vec![KeyInput::alt('e')]),
            OpenSystemEditor.into(),
        ),
    ])
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct KeyInputSequence(Vec<KeyInput>);

impl KeyInputSequence {
    pub fn new(keys: Vec<KeyInput>) -> Self {
        KeyInputSequence(keys)
    }
}

impl From<Vec<KeyInput>> for KeyInputSequence {
    fn from(keys: Vec<KeyInput>) -> Self {
        KeyInputSequence(keys)
    }
}

#[allow(deprecated)]
impl From<Vec<deprecated::KeyEvent>> for KeyInputSequence {
    fn from(events: Vec<deprecated::KeyEvent>) -> Self {
        KeyInputSequence(events.into_iter().map(|event| event.into()).collect())
    }
}

impl From<KeyInputSequence> for Vec<KeyInput> {
    fn from(seq: KeyInputSequence) -> Self {
        seq.0
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct KeyEventRegister {
    keys: Vec<KeyInput>,
    mode: EditorMode,
}

type RegisterCB = fn(&mut EditorState);

#[derive(Clone, Debug)]
struct RegisterVal(pub fn(&mut EditorState));

impl KeyEventRegister {
    pub fn new<T>(key: T, mode: EditorMode) -> Self
    where
        T: Into<KeyInputSequence>,
    {
        Self {
            keys: key.into().into(),
            mode,
        }
    }

    pub fn n<T>(key: T) -> Self
    where
        T: Into<KeyInputSequence>,
    {
        Self::new(key, EditorMode::Normal)
    }

    pub fn v<T>(key: T) -> Self
    where
        T: Into<KeyInputSequence>,
    {
        Self::new(key, EditorMode::Visual)
    }

    pub fn i<T>(key: T) -> Self
    where
        T: Into<KeyInputSequence>,
    {
        Self::new(key, EditorMode::Insert)
    }

    pub fn s<T>(key: T) -> Self
    where
        T: Into<KeyInputSequence>,
    {
        Self::new(key, EditorMode::Search)
    }
}

impl KeyEventHandler {
    pub(crate) fn on_event<T>(&mut self, key: T, state: &mut EditorState)
    where
        T: Into<KeyInput> + Copy + std::fmt::Debug,
    {
        let mode = state.mode;
        let key_input = key.into().normalize_altgr();

        // Always insert characters in insert mode
        if mode == EditorMode::Insert {
            if let input::KeyCode::Char(c) = key_input.key {
                if key_input.modifiers == input::Modifiers::NONE
                    || key_input.modifiers == input::Modifiers::SHIFT
                {
                    if self.capture_on_insert {
                        state.capture();
                    }
                    InsertChar(c).execute(state);
                    return;
                }
            }

            if matches!(key_input.key, input::KeyCode::Tab)
                && key_input.modifiers == input::Modifiers::NONE
            {
                if self.capture_on_insert {
                    state.capture();
                }
                InsertChar('\t').execute(state);
                return;
            }
        }

        // Always add characters to search in search mode
        if mode == EditorMode::Search {
            if let input::KeyCode::Char(c) = key_input.key {
                if key_input.modifiers == input::Modifiers::NONE {
                    AppendCharToSearch(c).execute(state);
                    return;
                }
            }
        }

        if matches!(mode, EditorMode::Normal | EditorMode::Visual)
            && self
                .vim_command_context()
                .handle_count_prefix(key_input, state)
        {
            return;
        }

        if mode == EditorMode::Normal
            && self
                .vim_command_context()
                .handle_normal_key(key_input, state)
        {
            return;
        }
        if mode == EditorMode::Visual
            && self
                .vim_command_context()
                .handle_visual_key(key_input, state)
        {
            return;
        }

        // Else lookup an action from the register
        if let Some(mut action) = self.get(key_input, mode) {
            let count = self.take_command_count();
            if count != 1 {
                apply_count(&mut action, count, key_input, mode, state);
            }
            action.execute(state);
            self.vim.clear();
        } else if matches!(mode, EditorMode::Normal | EditorMode::Visual) {
            if self.lookup.is_empty() {
                self.vim_command_context().clear_if_idle();
            } else {
                self.vim_command_context()
                    .capture_operator_count_if_needed();
            }
        }
    }

    fn vim_command_context(&mut self) -> VimCommandContext<'_> {
        VimCommandContext {
            state: &mut self.vim,
            lookup: &mut self.lookup,
        }
    }

    fn take_command_count(&mut self) -> usize {
        self.vim.take_command_count()
    }
}

fn apply_count(
    action: &mut Action,
    count: usize,
    key_input: KeyInput,
    mode: EditorMode,
    state: &mut EditorState,
) {
    match action {
        Action::MoveForward(a) => a.0 = count,
        Action::MoveBackward(a) => a.0 = count,
        Action::MoveUp(a) => a.0 = count,
        Action::MoveDown(a) => a.0 = count,
        Action::MoveWordForward(a) => a.0 = count,
        Action::MoveWordForwardToEndOfWord(a) => a.0 = count,
        Action::MoveWordBackward(a) => a.0 = count,
        Action::MoveBigWordForward(a) => a.0 = count,
        Action::MoveBigWordForwardToEndOfWord(a) => a.0 = count,
        Action::MoveBigWordBackward(a) => a.0 = count,
        Action::RemoveChar(a) => a.0 = count,
        Action::DeleteWordForward(a) => a.0 = count,
        Action::ChangeWordForward(a) => a.0 = count,
        Action::CopyWordForward(a) => a.0 = count,
        Action::DeleteToWordEnd(a) => a.0 = count,
        Action::ChangeToWordEnd(a) => a.0 = count,
        Action::CopyToWordEnd(a) => a.0 = count,
        Action::DeleteBigWordForward(a) => a.0 = count,
        Action::ChangeBigWordForward(a) => a.0 = count,
        Action::CopyBigWordForward(a) => a.0 = count,
        Action::DeleteToBigWordEnd(a) => a.0 = count,
        Action::ChangeToBigWordEnd(a) => a.0 = count,
        Action::CopyToBigWordEnd(a) => a.0 = count,
        Action::DeleteBigWordBackward(a) => a.0 = count,
        Action::ChangeBigWordBackward(a) => a.0 = count,
        Action::CopyBigWordBackward(a) => a.0 = count,
        Action::DeleteLineDown(a) => a.0 = count,
        Action::ChangeLineDown(a) => a.0 = count,
        Action::CopyLineDown(a) => a.0 = count,
        Action::DeleteLineUp(a) => a.0 = count,
        Action::ChangeLineUp(a) => a.0 = count,
        Action::CopyLineUp(a) => a.0 = count,
        Action::DeleteWordBackward(a) => a.0 = count,
        Action::ChangeWordBackward(a) => a.0 = count,
        Action::CopyWordBackward(a) => a.0 = count,
        Action::DeleteLine(a)
            if matches!(mode, EditorMode::Normal)
                && matches!(key_input.key, input::KeyCode::Char('d')) =>
        {
            a.0 = count;
        }
        Action::ChangeLine(a)
            if matches!(mode, EditorMode::Normal)
                && matches!(key_input.key, input::KeyCode::Char('c')) =>
        {
            a.0 = count;
        }
        Action::MoveToEndOfLine(_) => {
            for _ in 1..count {
                MoveDown(1).execute(state);
            }
        }
        Action::MoveToLastRow(_) => {
            if let Some(target_row) = count.checked_sub(1) {
                move_to_row(state, target_row);
            }
            // The counted G has already been handled; make the registered G a no-op.
            *action = MoveDown(0).into();
        }
        _ => {}
    }
}

fn move_to_row(state: &mut EditorState, target_row: usize) {
    let max_row = state.lines.len().saturating_sub(1);
    let target_row = target_row.min(max_row);
    let current_row = state.cursor.row;
    if target_row >= current_row {
        MoveDown(target_row - current_row).execute(state);
    } else {
        MoveUp(current_row - target_row).execute(state);
    }
}

#[cfg(test)]
mod tests {
    #[allow(deprecated)]
    use super::deprecated::KeyEvent;
    use super::*;
    use crate::clipboard::{ClipboardTrait, InternalClipboard};
    use ratatui_core::widgets::Widget;

    #[test]
    #[allow(deprecated)]
    fn test_key_event_register_with_key_event() {
        let register = KeyEventRegister::n(vec![KeyEvent::Ctrl('a'), KeyEvent::Char('b')]);
        assert_eq!(register.mode, EditorMode::Normal);
        assert_eq!(register.keys.len(), 2);

        assert_eq!(register.keys[0], KeyInput::ctrl('a'));
        assert_eq!(register.keys[1], KeyInput::new('b'));
    }

    #[test]
    fn test_key_event_register_with_key_input() {
        let register = KeyEventRegister::i(vec![KeyInput::ctrl('a'), KeyInput::new('b')]);
        assert_eq!(register.mode, EditorMode::Insert);
        assert_eq!(register.keys.len(), 2);

        assert_eq!(register.keys[0], KeyInput::ctrl('a'));
        assert_eq!(register.keys[1], KeyInput::new('b'));
    }

    #[test]
    fn test_key_event_register_with_crossterm() {
        use crossterm::event::{KeyCode as CTKeyCode, KeyEvent as CTKeyEvent, KeyModifiers};

        let ct_key_event = CTKeyEvent::new(CTKeyCode::Char('a'), KeyModifiers::CONTROL);
        let key_input: KeyInput = ct_key_event.into();

        let register = KeyEventRegister::v(vec![key_input, KeyInput::new(CTKeyCode::Enter)]);
        assert_eq!(register.mode, EditorMode::Visual);
        assert_eq!(register.keys.len(), 2);

        assert_eq!(register.keys[0], KeyInput::ctrl('a'));
        assert_eq!(register.keys[1], KeyInput::new(CTKeyCode::Enter));
    }

    #[test]
    fn test_insert_hello_world() {
        use crate::EditorState;

        let mut state = EditorState::default();
        state.mode = EditorMode::Insert;

        let mut handler = KeyEventHandler::default();

        let inputs = vec![
            KeyInput::shift('H'),
            KeyInput::new('e'),
            KeyInput::new('l'),
            KeyInput::new('l'),
            KeyInput::new('o'),
            KeyInput::new(' '),
            KeyInput::shift('W'),
            KeyInput::new('o'),
            KeyInput::new('r'),
            KeyInput::new('l'),
            KeyInput::new('d'),
            KeyInput::shift('!'),
            KeyInput::new(KeyCode::Enter),
            KeyInput::shift('H'),
            KeyInput::new('i'),
            KeyInput::shift('!'),
        ];

        for input in inputs {
            handler.on_event(input, &mut state);
        }

        assert_eq!(state.lines.to_string(), String::from("Hello World!\nHi!"));
    }

    #[test]
    fn test_altgr_normalization_inserts_characters() {
        use crate::EditorState;
        use crossterm::event::{KeyEvent as CTKeyEvent, KeyModifiers as CTMods};

        let mut state = EditorState::default();
        state.mode = EditorMode::Insert;

        let mut handler = KeyEventHandler::emacs_mode();

        // Simulate AltGr+[ (reported as Ctrl+Alt+[ on international keyboards)
        let altgr_bracket = CTKeyEvent::new(
            crossterm::event::KeyCode::Char('['),
            CTMods::CONTROL | CTMods::ALT,
        );
        handler.on_event(altgr_bracket, &mut state);

        // Simulate AltGr+]
        let altgr_bracket_close = CTKeyEvent::new(
            crossterm::event::KeyCode::Char(']'),
            CTMods::CONTROL | CTMods::ALT,
        );
        handler.on_event(altgr_bracket_close, &mut state);

        // Simulate AltGr+{ (with shift)
        let altgr_brace = CTKeyEvent::new(
            crossterm::event::KeyCode::Char('{'),
            CTMods::CONTROL | CTMods::ALT | CTMods::SHIFT,
        );
        handler.on_event(altgr_brace, &mut state);

        // Simulate AltGr+}
        let altgr_brace_close = CTKeyEvent::new(
            crossterm::event::KeyCode::Char('}'),
            CTMods::CONTROL | CTMods::ALT | CTMods::SHIFT,
        );
        handler.on_event(altgr_brace_close, &mut state);

        assert_eq!(state.lines.to_string(), "[]{}");
    }

    #[test]
    fn test_altgr_does_not_affect_letter_keybindings() {
        use crate::EditorState;

        let mut state = EditorState::new(crate::Lines::from("Hello World"));
        state.mode = EditorMode::Insert;

        let mut handler = KeyEventHandler::emacs_mode();

        // Alt+f should move forward word, not insert 'f'
        let alt_f = KeyInput::alt('f');
        handler.on_event(alt_f, &mut state);

        // Cursor should have moved to position 6 ('W'), not inserted 'f'
        assert_eq!(state.cursor.col, 6);
        assert_eq!(state.lines.to_string(), "Hello World");
    }

    fn press(handler: &mut KeyEventHandler, state: &mut EditorState, keys: &[KeyInput]) {
        for key in keys {
            handler.on_event(*key, state);
        }
    }

    #[test]
    fn vim_undo_redo_groups_insert_sessions_and_clears_redo_on_new_change() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("abc"));

        press(
            &mut handler,
            &mut state,
            &[
                KeyInput::new('i'),
                KeyInput::shift('X'),
                KeyInput::shift('Y'),
                KeyInput::new(KeyCode::Esc),
            ],
        );
        assert_eq!(state.lines.to_string(), "XYabc");
        assert_eq!(state.mode, EditorMode::Normal);

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "abc");

        press(&mut handler, &mut state, &[KeyInput::ctrl('r')]);
        assert_eq!(state.lines.to_string(), "XYabc");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "abc");

        press(
            &mut handler,
            &mut state,
            &[
                KeyInput::new('i'),
                KeyInput::shift('Z'),
                KeyInput::new(KeyCode::Esc),
            ],
        );
        assert_eq!(state.lines.to_string(), "Zabc");

        press(&mut handler, &mut state, &[KeyInput::ctrl('r')]);
        assert_eq!(state.lines.to_string(), "Zabc");
    }

    #[test]
    fn vim_search_navigation_and_cancel_are_not_undoable() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two\ntwo three\ntwo"));
        state.cursor = crate::Index2::new(0, 0);

        press(
            &mut handler,
            &mut state,
            &[
                KeyInput::new('/'),
                KeyInput::new('t'),
                KeyInput::new('w'),
                KeyInput::new('o'),
                KeyInput::new(KeyCode::Enter),
            ],
        );
        assert_eq!(state.mode, EditorMode::Normal);
        assert_eq!(state.cursor, crate::Index2::new(0, 4));

        press(&mut handler, &mut state, &[KeyInput::new('n')]);
        assert_eq!(state.cursor, crate::Index2::new(1, 0));

        press(&mut handler, &mut state, &[KeyInput::shift('N')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 4));

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two\ntwo three\ntwo");
        assert_eq!(state.cursor, crate::Index2::new(0, 4));

        press(&mut handler, &mut state, &[KeyInput::new('/')]);
        press(&mut handler, &mut state, &[KeyInput::new('t')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 4));
        press(&mut handler, &mut state, &[KeyInput::new(KeyCode::Esc)]);
        assert_eq!(state.mode, EditorMode::Normal);
        assert_eq!(state.cursor, crate::Index2::new(0, 4));
    }

    #[test]
    fn vim_reverse_search_uses_n_and_shift_n_like_vim() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two\ntwo three\ntwo"));
        state.cursor = crate::Index2::new(2, 2);

        press(
            &mut handler,
            &mut state,
            &[
                KeyInput::new('?'),
                KeyInput::new('t'),
                KeyInput::new('w'),
                KeyInput::new('o'),
                KeyInput::new(KeyCode::Enter),
            ],
        );
        assert_eq!(state.mode, EditorMode::Normal);
        assert_eq!(state.cursor, crate::Index2::new(2, 0));

        press(&mut handler, &mut state, &[KeyInput::new('n')]);
        assert_eq!(state.cursor, crate::Index2::new(1, 0));

        press(&mut handler, &mut state, &[KeyInput::shift('N')]);
        assert_eq!(state.cursor, crate::Index2::new(2, 0));
    }

    #[test]
    fn vim_word_motions_treat_underscore_as_word_char() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("test_1234"));
        state.cursor.col = 8;

        press(&mut handler, &mut state, &[KeyInput::new('b')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 0));

        press(&mut handler, &mut state, &[KeyInput::new('e')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 8));
    }

    #[test]
    fn vim_word_and_big_word_split_like_prompt_toolkit() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("foo.bar baz/qux"));

        press(&mut handler, &mut state, &[KeyInput::new('w')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 3));
        press(&mut handler, &mut state, &[KeyInput::new('w')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 4));

        state.cursor.col = 0;
        press(&mut handler, &mut state, &[KeyInput::shift('W')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 8));
        press(&mut handler, &mut state, &[KeyInput::shift('E')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 14));
        press(&mut handler, &mut state, &[KeyInput::shift('B')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 8));
    }

    #[test]
    fn vim_counts_apply_to_common_motions_and_edits() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two three four\nfive\nsix"));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('3'), KeyInput::new('w')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 14));
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('2'), KeyInput::new('b')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 4));
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('5'), KeyInput::new('l')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 9));
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('2'), KeyInput::new('$')],
        );
        assert_eq!(state.cursor, crate::Index2::new(1, 3));

        state.cursor = crate::Index2::new(0, 0);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('4'), KeyInput::new('x')],
        );
        assert_eq!(state.lines.to_string(), "two three four\nfive\nsix");
    }

    #[test]
    fn vim_operator_word_and_end_of_line_commands_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two three"));
        state.set_clipboard(InternalClipboard::default());

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "two three");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));
        assert_eq!(state.clip.get_text(), "one ");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two three");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "two three");
        assert_eq!(state.mode, EditorMode::Insert);

        state.mode = EditorMode::Normal;
        state = EditorState::new(crate::Lines::from("one two three"));
        state.set_clipboard(InternalClipboard::default());
        handler = KeyEventHandler::vim_mode();
        state.cursor.col = 4;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('$')],
        );
        assert_eq!(state.lines.to_string(), "one two three");
        assert_eq!(state.clip.get_text(), "two three");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('$')],
        );
        assert_eq!(state.lines.to_string(), "one ");
    }

    #[test]
    fn vim_invalid_partial_commands_clear_pending_state() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("abc(def)"));
        state.set_clipboard(InternalClipboard::default());

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('q')],
        );
        assert_eq!(state.lines.to_string(), "abc(def)");
        press(&mut handler, &mut state, &[KeyInput::new('l')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 1));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('i'), KeyInput::new('q')],
        );
        assert_eq!(state.lines.to_string(), "abc(def)");
        press(&mut handler, &mut state, &[KeyInput::new('l')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 2));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('t'), KeyInput::new('x')],
        );
        assert_eq!(state.lines.to_string(), "abc(def)");
        press(&mut handler, &mut state, &[KeyInput::new('l')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 3));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('2'), KeyInput::new('d'), KeyInput::new('q')],
        );
        press(&mut handler, &mut state, &[KeyInput::new('l')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 4));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('t'), KeyInput::new('x')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 4));
        press(&mut handler, &mut state, &[KeyInput::new('l')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 5));
    }

    #[test]
    fn vim_substitute_char_command_uses_change_operator() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("abcdef"));
        state.set_clipboard(InternalClipboard::default());

        press(&mut handler, &mut state, &[KeyInput::new('s')]);
        assert_eq!(state.lines.to_string(), "bcdef");
        assert_eq!(state.clip.get_text(), "a");
        assert_eq!(state.mode, EditorMode::Insert);

        state.mode = EditorMode::Normal;
        state = EditorState::new(crate::Lines::from("abcdef"));
        state.set_clipboard(InternalClipboard::default());
        handler = KeyEventHandler::vim_mode();
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('3'), KeyInput::new('s')],
        );
        assert_eq!(state.lines.to_string(), "def");
        assert_eq!(state.clip.get_text(), "abc");
        assert_eq!(state.mode, EditorMode::Insert);

        state.mode = EditorMode::Normal;
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "abcdef");
    }

    #[test]
    fn vim_normal_char_search_motions_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("foo(bar) baz)"));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('t'), KeyInput::new(')')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 6));

        state.cursor.col = 0;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('f'), KeyInput::new(')')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 7));

        state.cursor.col = 12;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::shift('T'), KeyInput::new('(')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 4));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::shift('F'), KeyInput::new('(')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 3));

        state.cursor.col = 0;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('2'), KeyInput::new('t'), KeyInput::new(')')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 11));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('t'), KeyInput::new('x')],
        );
        assert_eq!(state.cursor, crate::Index2::new(0, 11));
        press(&mut handler, &mut state, &[KeyInput::new('l')]);
        assert_eq!(state.cursor, crate::Index2::new(0, 12));
    }

    #[test]
    fn vim_operator_char_search_motions_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("foo(bar) baz"));
        state.set_clipboard(InternalClipboard::default());

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('t'), KeyInput::new(')')],
        );
        assert_eq!(state.lines.to_string(), ") baz");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));
        assert_eq!(state.clip.get_text(), "foo(bar");

        state = EditorState::new(crate::Lines::from("abcd("));
        state.set_clipboard(InternalClipboard::default());
        handler = KeyEventHandler::vim_mode();
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('t'), KeyInput::new('(')],
        );
        assert_eq!(state.lines.to_string(), "(");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));
        assert_eq!(state.clip.get_text(), "abcd");

        let area = ratatui_core::layout::Rect::new(0, 0, 10, 1);
        let mut buffer = ratatui_core::buffer::Buffer::empty(area);
        crate::EditorView::new(&mut state).render(area, &mut buffer);
        assert_eq!(
            state.cursor_screen_position(),
            Some(ratatui_core::layout::Position::new(0, 0))
        );

        state = EditorState::new(crate::Lines::from("foo(bar) baz"));
        state.set_clipboard(InternalClipboard::default());
        handler = KeyEventHandler::vim_mode();
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('f'), KeyInput::new(')')],
        );
        assert_eq!(state.lines.to_string(), "foo(bar) baz");
        assert_eq!(state.clip.get_text(), "foo(bar)");

        state.cursor.col = 8;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::shift('T'), KeyInput::new('(')],
        );
        assert_eq!(state.lines.to_string(), "foo(baz");
        assert_eq!(state.mode, EditorMode::Insert);
    }

    #[test]
    fn vim_operator_to_word_end_commands_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two"));
        state.set_clipboard(InternalClipboard::default());

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('e')],
        );
        assert_eq!(state.lines.to_string(), " two");
        assert_eq!(state.clip.get_text(), "one");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('e')],
        );
        assert_eq!(state.lines.to_string(), "one two");
        assert_eq!(state.clip.get_text(), "one");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::new('e')],
        );
        assert_eq!(state.lines.to_string(), " two");
        assert_eq!(state.mode, EditorMode::Insert);
    }

    #[test]
    fn vim_operator_backward_and_line_start_commands_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two three"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.col = 8;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('b')],
        );
        assert_eq!(state.lines.to_string(), "one three");
        assert_eq!(state.cursor, crate::Index2::new(0, 4));
        assert_eq!(state.clip.get_text(), "two ");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.col = 8;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('b')],
        );
        assert_eq!(state.lines.to_string(), "one two three");
        assert_eq!(state.clip.get_text(), "two ");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('0')],
        );
        assert_eq!(state.lines.to_string(), "three");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));
    }

    #[test]
    fn vim_operator_line_range_commands_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("a\nb\nc\nd"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.row = 1;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::shift('G')],
        );
        assert_eq!(state.lines.to_string(), "a\nb\nc\nd");
        assert_eq!(state.clip.get_text(), "\nb\nc\nd");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::shift('G')],
        );
        assert_eq!(state.lines.to_string(), "a");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.row = 2;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('g'), KeyInput::new('g')],
        );
        assert_eq!(state.lines.to_string(), "d");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.row = 1;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::shift('G')],
        );
        assert_eq!(state.lines.to_string(), "a");
        assert_eq!(state.mode, EditorMode::Insert);
    }

    #[test]
    fn vim_visual_text_objects_use_shared_ranges() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two three"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.col = 5;

        press(&mut handler, &mut state, &[KeyInput::new('v')]);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('i'), KeyInput::new('w')],
        );
        assert_eq!(state.mode, EditorMode::Visual);
        assert_eq!(
            state.selection,
            Some(crate::state::selection::Selection::new(
                crate::Index2::new(0, 4),
                crate::Index2::new(0, 6)
            ))
        );

        press(&mut handler, &mut state, &[KeyInput::new('y')]);
        assert_eq!(state.clip.get_text(), "two");
        assert_eq!(state.mode, EditorMode::Normal);

        state = EditorState::new(crate::Lines::from("foo(bar)"));
        state.cursor.col = 5;
        handler = KeyEventHandler::vim_mode();
        press(&mut handler, &mut state, &[KeyInput::new('v')]);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('a'), KeyInput::new(')')],
        );
        assert_eq!(
            state.selection,
            Some(crate::state::selection::Selection::new(
                crate::Index2::new(0, 3),
                crate::Index2::new(0, 7)
            ))
        );
    }

    #[test]
    fn vim_word_text_objects_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two three"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.col = 5;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('i'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "one two three");
        assert_eq!(state.clip.get_text(), "two");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('i'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "one  three");
        assert_eq!(state.cursor, crate::Index2::new(0, 4));
        assert_eq!(state.clip.get_text(), "two");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.col = 5;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('a'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "one three");
        assert_eq!(state.clip.get_text(), "two ");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.col = 5;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::new('i'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "one  three");
        assert_eq!(state.mode, EditorMode::Insert);
    }

    fn assert_cursor_in_bounds(state: &EditorState) {
        assert!(state.cursor.row < state.lines.len());
        let len = state.lines.len_col(state.cursor.row).unwrap_or_default();
        assert!(state.cursor.col <= len.max(1));
    }

    #[test]
    fn vim_undo_grouping_and_yank_invariants() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two three\nfour\nfive"));
        state.set_clipboard(InternalClipboard::default());

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('3'), KeyInput::new('w')],
        );
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two three\nfour\nfive");
        assert_cursor_in_bounds(&state);

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('w')],
        );
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two three\nfour\nfive");
        assert_cursor_in_bounds(&state);

        state.cursor = crate::Index2::new(0, 0);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('4'), KeyInput::new('x')],
        );
        assert_eq!(state.lines.to_string(), "two three\nfour\nfive");
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two three\nfour\nfive");
        press(&mut handler, &mut state, &[KeyInput::ctrl('r')]);
        assert_eq!(state.lines.to_string(), "two three\nfour\nfive");
        assert_cursor_in_bounds(&state);

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('3'), KeyInput::new('d'), KeyInput::new('d')],
        );
        assert_eq!(state.lines.to_string(), "");
        assert_cursor_in_bounds(&state);
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two three\nfour\nfive");
        assert_cursor_in_bounds(&state);

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('2'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "three\nfour\nfive");
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two three\nfour\nfive");
        assert_cursor_in_bounds(&state);
    }

    #[test]
    fn vim_cursor_clamps_after_destructive_line_commands() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("a\nb"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.row = 1;
        state.cursor.col = 1;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('d')],
        );
        assert_eq!(state.lines.to_string(), "a");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));
        assert_cursor_in_bounds(&state);

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('d')],
        );
        assert_eq!(state.lines.to_string(), "");
        assert_eq!(state.cursor, crate::Index2::new(0, 0));
        assert_cursor_in_bounds(&state);

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "a\nb");
        assert_cursor_in_bounds(&state);
    }

    #[test]
    fn vim_vertical_operator_motions_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("a\nb\nc\nd"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.row = 1;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('j')],
        );
        assert_eq!(state.clip.get_text(), "\nb\nc");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('j')],
        );
        assert_eq!(state.lines.to_string(), "a\nd");
        assert_eq!(state.cursor, crate::Index2::new(1, 0));

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.row = 2;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('k')],
        );
        assert_eq!(state.lines.to_string(), "a\nd");
        assert_eq!(state.cursor, crate::Index2::new(1, 0));

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.row = 1;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('2'), KeyInput::new('c'), KeyInput::new('j')],
        );
        assert_eq!(state.lines.to_string(), "a");
        assert_eq!(state.mode, EditorMode::Insert);
    }

    #[test]
    fn vim_big_word_operator_motions_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("foo.bar baz/qux"));
        state.set_clipboard(InternalClipboard::default());

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::shift('W')],
        );
        assert_eq!(state.lines.to_string(), "baz/qux");
        assert_eq!(state.clip.get_text(), "foo.bar ");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.col = 8;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::shift('E')],
        );
        assert_eq!(state.clip.get_text(), "baz/qux");

        state.cursor.col = 12;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::shift('B')],
        );
        assert_eq!(state.lines.to_string(), "foo.bar qux");
        assert_eq!(state.clip.get_text(), "baz/");
    }

    #[test]
    fn vim_big_word_text_objects_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("foo.bar baz"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.col = 1;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('i'), KeyInput::shift('W')],
        );
        assert_eq!(state.clip.get_text(), "foo.bar");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('a'), KeyInput::shift('W')],
        );
        assert_eq!(state.lines.to_string(), "baz");
        assert_eq!(state.clip.get_text(), "foo.bar ");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.col = 1;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::new('i'), KeyInput::shift('W')],
        );
        assert_eq!(state.lines.to_string(), " baz");
        assert_eq!(state.mode, EditorMode::Insert);
    }

    #[test]
    fn vim_delimiter_text_objects_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("foo(hello) bar"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.col = 5;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('y'), KeyInput::new('i'), KeyInput::new('(')],
        );
        assert_eq!(state.clip.get_text(), "hello");
        assert_eq!(state.lines.to_string(), "foo(hello) bar");

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('a'), KeyInput::new('(')],
        );
        assert_eq!(state.lines.to_string(), "foo bar");
        assert_eq!(state.clip.get_text(), "(hello)");

        state.mode = EditorMode::Normal;
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor.col = 5;
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::new('i'), KeyInput::new(')')],
        );
        assert_eq!(state.lines.to_string(), "foo() bar");
        assert_eq!(state.mode, EditorMode::Insert);
    }

    #[test]
    fn vim_change_line_commands_work() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("a\nb\nc\nd"));
        state.set_clipboard(InternalClipboard::default());
        state.cursor.row = 1;

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('c'), KeyInput::new('c')],
        );
        assert_eq!(state.lines.to_string(), "a\nc\nd");
        assert_eq!(state.cursor, crate::Index2::new(1, 0));
        assert_eq!(state.mode, EditorMode::Insert);
        assert_eq!(state.clip.get_text(), "\nb");

        state.mode = EditorMode::Normal;
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        state.cursor = crate::Index2::new(1, 0);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('2'), KeyInput::new('c'), KeyInput::new('c')],
        );
        assert_eq!(state.lines.to_string(), "a\nd");
        assert_eq!(state.mode, EditorMode::Insert);
        assert_eq!(state.clip.get_text(), "\nb\nc");

        state = EditorState::new(crate::Lines::from("abc def"));
        state.set_clipboard(InternalClipboard::default());
        handler = KeyEventHandler::vim_mode();
        state.cursor.col = 4;
        press(&mut handler, &mut state, &[KeyInput::shift('C')]);
        assert_eq!(state.lines.to_string(), "abc ");
        assert_eq!(state.mode, EditorMode::Insert);
        assert_eq!(state.clip.get_text(), "def");
    }

    #[test]
    fn vim_operator_counts_multiply_for_word_commands() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("one two three four"));

        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('2'), KeyInput::new('d'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "three four");

        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        press(
            &mut handler,
            &mut state,
            &[KeyInput::new('d'), KeyInput::new('2'), KeyInput::new('w')],
        );
        assert_eq!(state.lines.to_string(), "three four");

        state = EditorState::new(crate::Lines::from("one two three four five six"));
        state.set_clipboard(InternalClipboard::default());
        handler = KeyEventHandler::vim_mode();
        press(
            &mut handler,
            &mut state,
            &[
                KeyInput::new('2'),
                KeyInput::new('d'),
                KeyInput::new('3'),
                KeyInput::new('w'),
            ],
        );
        assert_eq!(state.lines.to_string(), "");
        press(&mut handler, &mut state, &[KeyInput::new('u')]);
        assert_eq!(state.lines.to_string(), "one two three four five six");
    }

    #[test]
    fn vim_count_g_moves_to_one_based_row_and_clamps() {
        let mut handler = KeyEventHandler::vim_mode();
        let mut state = EditorState::new(crate::Lines::from("a\nb\nc"));

        handler.on_event(KeyInput::new('2'), &mut state);
        handler.on_event(KeyInput::shift('G'), &mut state);
        assert_eq!(state.cursor.row, 1);

        handler.on_event(KeyInput::new('9'), &mut state);
        handler.on_event(KeyInput::shift('G'), &mut state);
        assert_eq!(state.cursor.row, 2);
    }
}
