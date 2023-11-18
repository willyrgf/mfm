use std::collections::HashMap;

use anyhow::{anyhow, Error};
use serde_json::Value;

use crate::state::Label;

pub trait Tracker {
    fn track(&mut self, index: Index, value: Value) -> Result<bool, Error>;
    fn recover(&self, index: Index) -> Result<Value, Error>;
    // TODO: may be this Label should be tag?
    // it may be an `search_by_tag`, and to do that we need
    // to carry the tags of an state inside the index
    // maybe and StateMetadata::from(state) can be an good idea,
    // StateMetadata should be Hashable, Cloneable and Copiable
    //
    fn search_by_label(&self, label: Label) -> Vec<Index>;
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Index(usize, Label);

impl Index {
    pub fn new(counter: usize, label: Label) -> Self {
        Self(counter, label)
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

    fn search_by_label(&self, label: Label) -> Vec<Index> {
        self.0
            .keys()
            .filter(|index| index.1 == label)
            .cloned()
            .collect()
    }
    // fn search(&self, label: Label) -> Option<&[Index]> {
    //     let ix = self.0.keys().filter(|index| index.1 == label).collect();
    // }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use crate::state::Label;

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
            Index::new(1, Label::new("value_one").unwrap()),
            Index::new(2, Label::new("value_two").unwrap()),
            Index::new(3, Label::new("value_three").unwrap()),
        ];

        for i in 0..indexes.len() {
            tracker
                .track(indexes[i].clone(), values[i].clone())
                .unwrap();
        }

        for i in 0..indexes.len() {
            let value_recovered = tracker.recover(indexes[i]).unwrap();
            assert_eq!(values[i], value_recovered);
        }
    }
}
