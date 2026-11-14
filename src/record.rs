//! Compact per-record field storage (docs/spec/slice-7.md).
//!
//! Records of one source share a single sorted list of field names, so each
//! record holds only its values. A record serializes exactly like the
//! name-keyed map it replaces, so evidence and byte counts do not change.
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};
use std::{collections::BTreeMap, sync::Arc};

/// Shared, sorted field names of one source's records.
pub type FieldNames = Arc<Vec<String>>;

/// Values keyed by field name, in ascending name order.
///
/// Invariant: `values[i]` belongs to `names[i]` for every `i < values.len()`,
/// and `names` is sorted. Records built in name order share `names`; a record
/// that gains a name outside its shared list gets a private copy.
#[derive(Debug, Clone)]
pub struct FieldMap<V> {
    names: FieldNames,
    values: Vec<V>,
}

impl<V> Default for FieldMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> FieldMap<V> {
    pub fn new() -> Self {
        Self {
            names: Arc::new(Vec::new()),
            values: Vec::new(),
        }
    }
    /// An empty record that will hold values for `names`, inserted in order.
    pub fn with_names(names: FieldNames) -> Self {
        let values = Vec::with_capacity(names.len());
        Self { names, values }
    }
    fn position(&self, name: &str) -> Result<usize, usize> {
        self.names[..self.values.len()].binary_search_by(|n| n.as_str().cmp(name))
    }
    pub fn get(&self, name: &str) -> Option<&V> {
        self.position(name).ok().map(|i| &self.values[i])
    }
    pub fn get_mut(&mut self, name: &str) -> Option<&mut V> {
        self.position(name).ok().map(|i| &mut self.values[i])
    }
    pub fn contains_key(&self, name: &str) -> bool {
        self.position(name).is_ok()
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.names.iter().zip(&self.values)
    }
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.names[..self.values.len()].iter()
    }
    pub fn values(&self) -> std::slice::Iter<'_, V> {
        self.values.iter()
    }
    /// Set `name` to `value`, returning the previous value.
    pub fn insert(&mut self, name: String, value: V) -> Option<V> {
        let next = self.values.len();
        // Fast path: the next shared name, as adapters fill records in order.
        if self.names.get(next) == Some(&name) {
            self.values.push(value);
            return None;
        }
        match self.position(&name) {
            Ok(i) => Some(std::mem::replace(&mut self.values[i], value)),
            Err(i) => {
                // A name outside the filled shared prefix: take a private list.
                let names = Arc::make_mut(&mut self.names);
                names.truncate(self.values.len());
                names.insert(i, name);
                self.values.insert(i, value);
                None
            }
        }
    }
}

impl<V> std::ops::Index<&str> for FieldMap<V> {
    type Output = V;
    /// Like a map, panics when the record has no field `name`.
    fn index(&self, name: &str) -> &V {
        self.get(name)
            .unwrap_or_else(|| panic!("record has no field {name:?}"))
    }
}

impl<V: PartialEq> PartialEq for FieldMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl<V> FromIterator<(String, V)> for FieldMap<V> {
    fn from_iter<I: IntoIterator<Item = (String, V)>>(iter: I) -> Self {
        let sorted: BTreeMap<String, V> = iter.into_iter().collect();
        let (names, values): (Vec<String>, Vec<V>) = sorted.into_iter().unzip();
        Self {
            names: Arc::new(names),
            values,
        }
    }
}

impl<V> From<BTreeMap<String, V>> for FieldMap<V> {
    fn from(map: BTreeMap<String, V>) -> Self {
        map.into_iter().collect()
    }
}

impl<V: Clone> From<&FieldMap<V>> for BTreeMap<String, V> {
    fn from(map: &FieldMap<V>) -> Self {
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

impl<V: Serialize> Serialize for FieldMap<V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (k, v) in self.iter() {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for FieldMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        BTreeMap::<String, V>::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_names_fill_in_order_and_serialize_like_a_map() {
        let names: FieldNames = Arc::new(vec!["a".into(), "b".into(), "c".into()]);
        let mut first = FieldMap::with_names(names.clone());
        let mut second = FieldMap::with_names(names.clone());
        for (k, v) in [("a", 1), ("b", 2), ("c", 3)] {
            first.insert(k.into(), v);
            second.insert(k.into(), v * 10);
        }
        assert!(Arc::ptr_eq(&first.names, &second.names));
        assert_eq!(first.get("b"), Some(&2));
        assert_eq!(second.get("c"), Some(&30));
        assert_eq!(first.get("z"), None);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            r#"{"a":1,"b":2,"c":3}"#
        );
    }

    #[test]
    fn a_new_name_takes_a_private_sorted_list() {
        let names: FieldNames = Arc::new(vec!["a".into(), "c".into()]);
        let mut shared = FieldMap::with_names(names.clone());
        shared.insert("a".into(), 1);
        shared.insert("c".into(), 3);
        let mut extended = shared.clone();
        assert_eq!(extended.insert("b".into(), 2), None);
        assert_eq!(extended.insert("c".into(), 4), Some(3));
        assert_eq!(
            serde_json::to_string(&extended).unwrap(),
            r#"{"a":1,"b":2,"c":4}"#
        );
        assert_eq!(serde_json::to_string(&shared).unwrap(), r#"{"a":1,"c":3}"#);
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn a_partly_filled_record_sees_only_its_values() {
        let names: FieldNames = Arc::new(vec!["a".into(), "b".into()]);
        let mut partial = FieldMap::with_names(names);
        partial.insert("a".into(), 1);
        assert!(!partial.contains_key("b"));
        assert_eq!(partial.len(), 1);
        assert_eq!(partial.keys().collect::<Vec<_>>(), ["a"]);
        // Out of order: a later name inserted before an earlier one.
        let mut reversed = FieldMap::with_names(Arc::new(vec!["a".into(), "b".into()]));
        reversed.insert("b".into(), 2);
        reversed.insert("a".into(), 1);
        assert_eq!(
            serde_json::to_string(&reversed).unwrap(),
            r#"{"a":1,"b":2}"#
        );
    }

    #[test]
    fn deserializes_from_a_map() {
        let map: FieldMap<i32> = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        assert_eq!(
            map.iter().collect::<Vec<_>>(),
            [(&"a".to_string(), &1), (&"b".to_string(), &2)]
        );
    }
}
