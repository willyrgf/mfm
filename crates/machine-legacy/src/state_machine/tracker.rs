use std::{collections::HashMap, fmt::Debug};

use anyhow::{Error, Result};
use serde_json::Value;

use crate::state::{safe_context::SafeContext, Label, Tag};

pub trait TrackerMetadata {
    fn indexes(&self) -> Vec<Index>;
    fn search_by_tag(&self, tag: &Tag) -> Vec<Index>;
    fn search_by_index(&self, index: &usize) -> Option<Index>;
    fn history(&self) -> TrackerHistory;
}

#[derive(Clone)]
pub struct TrackerHistory(Vec<(usize, Index, Value)>);

impl Debug for TrackerHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_list = f.debug_list();
        for (step, idx, value) in &self.0 {
            debug_list.entry(&format!(
                "Step {} - State index: {}, Label: {}, Tags: {:?}, Context: {}",
                step, idx.state_index, idx.state_label, idx.state_tags, value
            ));
        }
        debug_list.finish()
    }
}

impl TrackerHistory {
    pub fn new(v: Vec<(usize, Index, Value)>) -> Self {
        Self(v)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, index: Index, context: SafeContext) {
        let snapshot = context.snapshot().unwrap_or(context.clone());
        if let Ok(value) = snapshot.dump() {
            self.0.push((self.0.len(), index, value));
        }
    }
}

impl IntoIterator for TrackerHistory {
    type Item = (usize, Index, Value);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

pub trait Tracker: TrackerMetadata {
    fn track(&mut self, index: Index, context: SafeContext) -> Result<bool, Error>;
    fn recover(&self, index: Index) -> Option<SafeContext>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Index {
    pub state_index: usize,
    pub state_label: Label,
    pub state_tags: Vec<Tag>,
}

impl Index {
    pub fn new(state_index: usize, state_label: Label, state_tags: Vec<Tag>) -> Self {
        Self {
            state_index,
            state_label,
            state_tags,
        }
    }
}

pub struct HashMapTracker {
    tracker: HashMap<Index, SafeContext>,
    history: TrackerHistory,
}

impl HashMapTracker {
    pub fn new() -> Self {
        Self {
            tracker: HashMap::new(),
            history: TrackerHistory::new(Vec::new()),
        }
    }
}

impl Default for HashMapTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker for HashMapTracker {
    fn track(&mut self, index: Index, context: SafeContext) -> Result<bool, Error> {
        // Store a deep snapshot so recoveries restore the context at the time of tracking.
        let snapshot = context.snapshot().unwrap_or(context);
        self.history.push(index.clone(), snapshot.clone());
        self.tracker.insert(index, snapshot);
        Ok(true)
    }

    fn recover(&self, index: Index) -> Option<SafeContext> {
        self.tracker.get(&index).cloned()
    }
}

impl TrackerMetadata for HashMapTracker {
    fn search_by_tag(&self, tag: &Tag) -> Vec<Index> {
        self.tracker
            .keys()
            .filter(|index| index.state_tags.contains(tag))
            .cloned()
            .collect()
    }

    fn search_by_index(&self, state_index: &usize) -> Option<Index> {
        self.tracker
            .keys()
            .find(|index| &index.state_index == state_index)
            .cloned()
    }

    fn indexes(&self) -> Vec<Index> {
        self.tracker.keys().cloned().collect()
    }

    fn history(&self) -> TrackerHistory {
        self.history.clone()
    }
}

// Implement TrackerMetadata for Box<dyn Tracker>
impl TrackerMetadata for Box<dyn Tracker> {
    fn indexes(&self) -> Vec<Index> {
        (**self).indexes()
    }

    fn search_by_tag(&self, tag: &Tag) -> Vec<Index> {
        (**self).search_by_tag(tag)
    }

    fn search_by_index(&self, index: &usize) -> Option<Index> {
        (**self).search_by_index(index)
    }

    fn history(&self) -> TrackerHistory {
        (**self).history()
    }
}

// Implement Tracker for Box<dyn Tracker>
impl Tracker for Box<dyn Tracker> {
    fn track(&mut self, index: Index, context: SafeContext) -> Result<bool, Error> {
        (**self).track(index, context)
    }

    fn recover(&self, index: Index) -> Option<SafeContext> {
        (**self).recover(index)
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use crate::state::{safe_context::create_default_safe_context, Label, Tag};

    use super::{HashMapTracker, Index, Tracker};

    #[test]
    fn test_tracker() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        // Create test contexts with SafeContext
        let context1 = create_default_safe_context();
        context1.write_value("value", &json!(1)).unwrap();

        let context2 = create_default_safe_context();
        context2.write_value("value", &json!(2)).unwrap();

        let context3 = create_default_safe_context();
        context3.write_value("value", &json!(3)).unwrap();

        let contexts = [context1, context2, context3];

        let indexes = [
            Index::new(
                1,
                Label::new("value_one").unwrap(),
                vec![Tag::new("tag_one").unwrap()],
            ),
            Index::new(
                2,
                Label::new("value_two").unwrap(),
                vec![Tag::new("tag_two").unwrap()],
            ),
            Index::new(
                3,
                Label::new("value_three").unwrap(),
                vec![Tag::new("tag_three").unwrap()],
            ),
        ];

        for i in 0..indexes.len() {
            tracker
                .track(indexes[i].clone(), contexts[i].clone())
                .unwrap();
        }

        for i in 0..indexes.len() {
            let context_recovered = tracker.recover(indexes[i].clone()).unwrap();

            let value_recovered = context_recovered.read_value("value").unwrap();
            let value_expected = contexts[i].read_value("value").unwrap();

            assert_eq!(value_expected, value_recovered);
        }
    }

    #[test]
    fn test_search_by_tag() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        // Create test contexts with SafeContext
        let context1 = create_default_safe_context();
        context1.write_value("value", &json!(1)).unwrap();

        let context2 = create_default_safe_context();
        context2.write_value("value", &json!(2)).unwrap();

        let context3 = create_default_safe_context();
        context3.write_value("value", &json!(3)).unwrap();

        let contexts = [context1, context2, context3];

        let indexes = [
            Index::new(
                1,
                Label::new("value_one").unwrap(),
                vec![Tag::new("tag_one").unwrap()],
            ),
            Index::new(
                2,
                Label::new("value_two").unwrap(),
                vec![Tag::new("tag_two").unwrap(), Tag::new("tag_one").unwrap()],
            ),
            Index::new(
                3,
                Label::new("value_three").unwrap(),
                vec![Tag::new("tag_three").unwrap(), Tag::new("tag_one").unwrap()],
            ),
        ];

        for i in 0..indexes.len() {
            tracker
                .track(indexes[i].clone(), contexts[i].clone())
                .unwrap();
        }

        let indexes_by_tag = tracker.search_by_tag(&Tag::new("tag_two").unwrap());

        assert_eq!(indexes_by_tag.len(), 1);
        assert_eq!(
            indexes_by_tag.first().unwrap().state_label,
            Label::new("value_two").unwrap()
        );

        let search_results = tracker.search_by_tag(&Tag::new("tag_one").unwrap());

        // Should find 3 states with tag_one
        assert_eq!(search_results.len(), 3);
    }

    #[test]
    fn test_list() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        // Create test contexts with SafeContext
        let context1 = create_default_safe_context();
        context1.write_value("value", &json!(1)).unwrap();

        let context2 = create_default_safe_context();
        context2.write_value("value", &json!(2)).unwrap();

        let context3 = create_default_safe_context();
        context3.write_value("value", &json!(3)).unwrap();

        let contexts = [context1, context2, context3];

        let indexes = [
            Index::new(
                1,
                Label::new("value_one").unwrap(),
                vec![Tag::new("tag_one").unwrap()],
            ),
            Index::new(
                2,
                Label::new("value_two").unwrap(),
                vec![Tag::new("tag_two").unwrap()],
            ),
            Index::new(
                3,
                Label::new("value_three").unwrap(),
                vec![Tag::new("tag_three").unwrap()],
            ),
        ];

        for i in 0..indexes.len() {
            tracker
                .track(indexes[i].clone(), contexts[i].clone())
                .unwrap();
        }

        let tracker_indexes = tracker.indexes();

        assert_eq!(tracker_indexes.len(), 3);
    }
}
