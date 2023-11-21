use std::collections::HashMap;

use anyhow::{anyhow, Error};
use serde_json::Value;

use crate::state::{Label, Tag};

pub trait Tracker {
    fn track(&mut self, index: Index, value: Value) -> Result<bool, Error>;
    fn recover(&self, index: Index) -> Result<Value, Error>;
    // TODO: may be this Label should be tag?
    // it may be an `search_by_tag`, and to do that we need
    // to carry the tags of an state inside the index
    // maybe and StateMetadata::from(state) can be an good idea,
    // StateMetadata should be Hashable, Cloneable and Copiable
    //
    fn search_by_tag(&self, tag: &Tag) -> Vec<Index>;
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Index {
    state_index: usize,
    state_label: Label,
    state_tags: Vec<Tag>,
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

pub struct HashMapTracker(HashMap<Index, Value>);

impl HashMapTracker {
    pub fn new() -> Self {
        HashMapTracker(HashMap::new())
    }
}

impl Tracker for HashMapTracker {
    fn track(&mut self, index: Index, value: Value) -> Result<bool, Error> {
        Ok(self.0.insert(index, value).is_none())
    }

    fn recover(&self, index: Index) -> Result<Value, Error> {
        Ok(self
            .0
            .get(&index)
            .cloned()
            .clone()
            .ok_or(anyhow!("index not found"))?)
    }

    fn search_by_tag(&self, tag: &Tag) -> Vec<Index> {
        self.0
            .keys()
            .filter(|index| index.state_tags.contains(tag))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use crate::state::{Label, Tag};

    use super::{HashMapTracker, Index, Tracker};

    #[test]
    fn test_tracker() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        let values = vec![
            json!({"value": 1}),
            json!({"value": 2}),
            json!({"value": 3}),
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
                .track(indexes[i].clone(), values[i].clone())
                .unwrap();
        }

        for i in 0..indexes.len() {
            let value_recovered = tracker.recover(indexes[i].clone()).unwrap();
            assert_eq!(values[i], value_recovered);
        }
    }

    #[test]
    fn test_search_by_tag() {
        let tracker: &mut dyn Tracker = &mut HashMapTracker::new();

        let values = vec![
            json!({"value": 1}),
            json!({"value": 2}),
            json!({"value": 3}),
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
                .track(indexes[i].clone(), values[i].clone())
                .unwrap();
        }

        let indexes_by_tag = tracker.search_by_tag(&Tag::new("tag_two").unwrap());

        assert_eq!(indexes_by_tag.len(), 1);
        assert_eq!(
            indexes_by_tag.first().unwrap().state_label,
            Label::new("value_two").unwrap()
        );
    }
}
