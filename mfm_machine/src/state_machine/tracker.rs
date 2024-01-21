use std::{collections::HashMap, fmt::Debug};

use anyhow::{anyhow, Error};
use serde_json::Value;

use crate::state::{context::ContextWrapper, Label, Tag};

pub trait TrackerMetadata {
    fn indexes(&self) -> Vec<Index>;
    fn search_by_tag(&self, tag: &Tag) -> Vec<Index>;
    fn history(&self) -> TrackerHistory;
}

#[derive(Default, Clone)]
pub struct TrackerHistory(Vec<(usize, Index, Value)>);

impl Debug for TrackerHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: add a way to see the context at tracker history
        self.0.iter().try_for_each(|(history_id, index, value)| {
            writeln!(
                f,
                "history_id ({}); index ({:?}); context ({:?})",
                history_id, index, value
            )
        })
    }
}

impl TrackerHistory {
    pub fn new(v: Vec<(usize, Index, Value)>) -> Self {
        TrackerHistory(v)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, index: Index, context: ContextWrapper) {
        self.0
            .push((self.len(), index, context.lock().unwrap().dump().unwrap()))
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
    fn track(&mut self, index: Index, context: ContextWrapper) -> Result<bool, Error>;
    fn recover(&self, index: Index) -> Result<ContextWrapper, Error>;
}

// TODO: should it be public? may export methods to access it
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
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
    tracker: HashMap<Index, ContextWrapper>,
    history: TrackerHistory,
}

impl HashMapTracker {
    pub fn new() -> Self {
        Self {
            tracker: HashMap::new(),
            history: TrackerHistory::default(),
        }
    }
}

impl Default for HashMapTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracker for HashMapTracker {
    // TODO: add validations
    fn track(&mut self, index: Index, context: ContextWrapper) -> Result<bool, Error> {
        self.history.push(index.clone(), context.clone());
        Ok(self.tracker.insert(index, context).is_none())
    }

    fn recover(&self, index: Index) -> Result<ContextWrapper, Error> {
        self.tracker
            .get(&index)
            .cloned()
            .clone()
            .ok_or(anyhow!("index not found"))
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

    fn indexes(&self) -> Vec<Index> {
        self.tracker.keys().cloned().collect()
    }

    fn history(&self) -> TrackerHistory {
        self.history.clone()
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use serde_json::json;

    use crate::state::{
        context::{wrap_context, ContextWrapper, Local},
        Label, Tag,
    };

    use super::{HashMapTracker, Index, Tracker};

    #[test]
    fn test_tracker() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        let contexts: Vec<ContextWrapper> = vec![
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(1))]))),
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(2))]))),
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(3))]))),
        ];
        let indexes = vec![
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

            let value_recovered = context_recovered.lock().unwrap().dump().unwrap();
            let value_expected = contexts[i].lock().unwrap().dump().unwrap();

            assert_eq!(value_expected, value_recovered);
        }
    }

    #[test]
    fn test_search_by_tag() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        let contexts: Vec<ContextWrapper> = vec![
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(1))]))),
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(2))]))),
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(3))]))),
        ];
        let indexes = vec![
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

        let indexes_by_tag = tracker.search_by_tag(&Tag::new("tag_two").unwrap());

        assert_eq!(indexes_by_tag.len(), 1);
        assert_eq!(
            indexes_by_tag.first().unwrap().state_label,
            Label::new("value_two").unwrap()
        );
    }

    #[test]
    fn test_list() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        let contexts: Vec<ContextWrapper> = vec![
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(1))]))),
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(2))]))),
            wrap_context(Local::new(HashMap::from([("value".to_string(), json!(3))]))),
        ];
        let indexes = vec![
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

        let indexes = tracker.indexes();

        assert_eq!(indexes.len(), 3);

        println!("indexes: {:?}", indexes);
    }
}
