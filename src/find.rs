//! Literal, case-sensitive find bar state. An empty needle does not change the buffer.

use crate::buffer::Buffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindField {
    Needle,
    Replacement,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindQuery {
    needle: String,
    replacement: String,
    field: FindField,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindEffect {
    Unchanged,
    Missing,
    Found,
    ReplacedOne,
    ReplacedAll(usize),
}

impl FindQuery {
    pub fn new() -> Self {
        Self {
            needle: String::new(),
            replacement: String::new(),
            field: FindField::Needle,
        }
    }

    pub fn needle(&self) -> &str {
        &self.needle
    }

    pub fn replacement(&self) -> &str {
        &self.replacement
    }

    pub fn field(&self) -> FindField {
        self.field
    }

    pub fn focus_field(&mut self, field: FindField) {
        self.field = field;
    }

    pub fn focus_needle(&mut self) {
        self.field = FindField::Needle;
    }

    pub fn toggle_field(&mut self) {
        self.field = match self.field {
            FindField::Needle => FindField::Replacement,
            FindField::Replacement => FindField::Needle,
        };
    }

    pub fn insert(&mut self, text: &str) {
        self.current_mut().push_str(text);
    }

    pub fn backspace(&mut self) {
        self.current_mut().pop();
    }

    pub fn find_next(&self, buffer: &mut Buffer) -> FindEffect {
        if self.needle.is_empty() {
            return FindEffect::Unchanged;
        }
        if buffer.find_next(&self.needle) {
            FindEffect::Found
        } else {
            FindEffect::Missing
        }
    }

    pub fn replace_one(&self, buffer: &mut Buffer) -> FindEffect {
        if self.needle.is_empty() {
            return FindEffect::Unchanged;
        }
        if buffer.replace_one(&self.needle, &self.replacement) {
            FindEffect::ReplacedOne
        } else {
            FindEffect::Missing
        }
    }

    pub fn replace_all(&self, buffer: &mut Buffer) -> FindEffect {
        if self.needle.is_empty() {
            return FindEffect::Unchanged;
        }
        let count = buffer.replace_all(&self.needle, &self.replacement);
        if count == 0 {
            FindEffect::Missing
        } else {
            FindEffect::ReplacedAll(count)
        }
    }

    fn current_mut(&mut self) -> &mut String {
        match self.field {
            FindField::Needle => &mut self.needle,
            FindField::Replacement => &mut self.replacement,
        }
    }
}

impl FindEffect {
    pub fn redraws(self) -> bool {
        matches!(self, Self::Found | Self::ReplacedOne | Self::ReplacedAll(_))
    }

    pub fn status(self) -> Option<String> {
        match self {
            Self::Unchanged => None,
            Self::Missing => Some("No matches".to_string()),
            Self::Found => Some("Found".to_string()),
            Self::ReplacedOne => Some("Replaced".to_string()),
            Self::ReplacedAll(count) => Some(format!("Replaced {count}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_edits_only_the_focused_field() {
        let mut query = FindQuery::new();
        assert_eq!(query.field(), FindField::Needle);
        assert_eq!(query.needle(), "");
        assert_eq!(query.replacement(), "");

        query.insert("a");
        query.insert("α");
        assert_eq!(query.needle(), "aα");
        assert_eq!(query.replacement(), "");

        query.toggle_field();
        assert_eq!(query.field(), FindField::Replacement);
        query.insert("b");
        assert_eq!(query.needle(), "aα");
        assert_eq!(query.replacement(), "b");
        query.backspace();
        query.backspace();
        assert_eq!(query.replacement(), "");
        assert_eq!(query.needle(), "aα");

        query.focus_field(FindField::Needle);
        query.backspace();
        assert_eq!(query.needle(), "a");
        query.focus_needle();
        assert_eq!(query.field(), FindField::Needle);
    }

    #[test]
    fn empty_needle_does_not_change_the_buffer() {
        let mut buffer = Buffer::new("alpha");
        buffer.set_cursor(2, false);
        let selection = buffer.selection();
        let mut query = FindQuery::new();
        query.focus_field(FindField::Replacement);
        query.insert("gone");

        let find = query.find_next(&mut buffer);
        let once = query.replace_one(&mut buffer);
        let all = query.replace_all(&mut buffer);
        assert_eq!(find, FindEffect::Unchanged);
        assert_eq!(once, FindEffect::Unchanged);
        assert_eq!(all, FindEffect::Unchanged);
        assert!(find.status().is_none());
        assert!(!find.redraws());
        assert_eq!(buffer.text(), "alpha");
        assert_eq!(buffer.selection(), selection);
    }

    #[test]
    fn find_wraps_and_a_miss_leaves_the_selection() {
        let mut query = FindQuery::new();
        query.insert("alpha");
        let mut buffer = Buffer::new("alpha beta alpha");

        assert_eq!(query.find_next(&mut buffer), FindEffect::Found);
        assert_eq!(buffer.selection(), 0..5);
        assert_eq!(buffer.selected_text(), "alpha");
        assert_eq!(query.find_next(&mut buffer), FindEffect::Found);
        assert_eq!(buffer.selection(), 11..16);
        assert_eq!(query.find_next(&mut buffer), FindEffect::Found);
        assert_eq!(buffer.selection(), 0..5);
        assert_eq!(FindEffect::Found.status().as_deref(), Some("Found"));
        assert!(FindEffect::Found.redraws());

        buffer.set_cursor(buffer.len(), false);
        assert_eq!(query.find_next(&mut buffer), FindEffect::Found);
        assert_eq!(buffer.selection(), 0..5);

        let parked = buffer.selection();
        let mut dotted = FindQuery::new();
        dotted.insert("a.c");
        assert_eq!(dotted.find_next(&mut buffer), FindEffect::Missing);
        assert_eq!(buffer.selection(), parked);
        assert_eq!(buffer.text(), "alpha beta alpha");
        assert_eq!(FindEffect::Missing.status().as_deref(), Some("No matches"));
        assert!(!FindEffect::Missing.redraws());

        let mut upper = FindQuery::new();
        upper.insert("ALPHA");
        assert_eq!(upper.find_next(&mut buffer), FindEffect::Missing);
        assert_eq!(upper.replace_one(&mut buffer), FindEffect::Missing);
        assert_eq!(upper.replace_all(&mut buffer), FindEffect::Missing);
        assert_eq!(buffer.text(), "alpha beta alpha");
        assert_eq!(buffer.selection(), parked);
    }

    #[test]
    fn replace_one_and_replace_all_use_the_literal_strings() {
        let mut query = FindQuery::new();
        query.insert("alpha");
        query.focus_field(FindField::Replacement);
        query.insert("a");

        let mut buffer = Buffer::new("alpha alpha");
        assert_eq!(query.replace_one(&mut buffer), FindEffect::ReplacedOne);
        assert_eq!(buffer.text(), "a alpha");
        assert_eq!(
            FindEffect::ReplacedOne.status().as_deref(),
            Some("Replaced")
        );

        assert_eq!(query.replace_all(&mut buffer), FindEffect::ReplacedAll(1));
        assert_eq!(buffer.text(), "a a");
        assert_eq!(
            FindEffect::ReplacedAll(1).status().as_deref(),
            Some("Replaced 1")
        );
        assert!(FindEffect::ReplacedAll(1).redraws());

        let mut same = FindQuery::new();
        same.insert("a");
        same.focus_field(FindField::Replacement);
        same.insert("a");
        assert_eq!(same.replace_all(&mut buffer), FindEffect::ReplacedAll(2));
        assert_eq!(buffer.text(), "a a");
    }
}
