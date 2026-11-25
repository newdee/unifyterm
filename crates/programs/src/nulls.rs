//! JSON that another program wrote: a member it gives as `null` is read
//! as one it left out.

use serde::{Deserialize, Deserializer};

/// For `#[serde(deserialize_with = "or_empty")]`: a `null` is the field's
/// empty value.
///
/// `#[serde(default)]` covers only a member that is left out. One that is
/// there as `null` is an error without this, and the error takes the whole
/// document with it: every card of a list, for one figure that is not known.
///
/// # Errors
///
/// When the member is neither `null` nor of the field's type.
pub fn or_empty<'de, D, T>(from: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(from)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::or_empty;

    #[derive(Deserialize, Debug, PartialEq)]
    struct Read {
        #[serde(default, deserialize_with = "or_empty")]
        name: String,
        #[serde(default, deserialize_with = "or_empty")]
        words: Vec<String>,
        #[serde(deserialize_with = "or_empty")]
        needed: u64,
    }

    #[test]
    fn a_null_is_the_empty_value_and_a_wrong_type_is_still_an_error() {
        let given = |text: &str| serde_json::from_str::<Read>(text).map_err(|error| error.to_string());
        let empty = Read { name: String::new(), words: Vec::new(), needed: 0 };
        assert_eq!(given(r#"{"name": null, "words": null, "needed": null}"#), Ok(empty), "null: as left out");
        assert_eq!(
            given(r#"{"name": "a", "words": ["b"], "needed": 3}"#),
            Ok(Read { name: "a".into(), words: vec!["b".into()], needed: 3 }),
            "what is given is read as before"
        );
        assert_eq!(given(r#"{"needed": 1}"#).map(|read| read.name), Ok(String::new()), "left out: the default");
        assert!(given(r#"{"name": "a"}"#).is_err_and(|error| error.contains("missing field `needed`")));
        assert!(given(r#"{"name": 3, "needed": 1}"#).is_err_and(|error| error.contains("expected a string")));
        assert!(given(r#"{"words": [null], "needed": 1}"#).is_err(), "a null inside a list is no word");
    }
}
