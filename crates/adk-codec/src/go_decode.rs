use serde::Deserialize;

pub(crate) struct IntegerSeed<'a, T>(pub(crate) &'a mut T);
impl<'de, T: std::str::FromStr> serde::de::DeserializeSeed<'de> for IntegerSeed<'_, T>
where
    T::Err: std::fmt::Display,
{
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        let value: Option<Box<serde_json::value::RawValue>> = serde::Deserialize::deserialize(d)?;
        if let Some(value) = value {
            // Serde classifies -0 as a float, but Go accepts it for integer fields.
            *self.0 = value.get().parse().map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}

// Go retains backing-array elements across repeated nonempty slice fields.
#[derive(Default)]
pub(crate) struct GoSlice<T> {
    slots: Vec<T>,
    len: Option<usize>,
}
impl<T> GoSlice<T> {
    pub(crate) fn from_values(slots: Vec<T>) -> Self {
        Self {
            len: Some(slots.len()),
            slots,
        }
    }
    pub(crate) fn into_values(self) -> Option<Vec<T>> {
        self.len
            .map(|len| self.slots.into_iter().take(len).collect())
    }
}
pub(crate) struct ElementSeed<'a, T>(pub(crate) &'a mut T);
impl<'de, T: Deserialize<'de>> serde::de::DeserializeSeed<'de> for ElementSeed<'_, T> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_option(self)
    }
}
impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for ElementSeed<'_, T> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a nullable SDK field")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        T::deserialize_in_place(d, self.0)
    }
}
impl<'de, T: Deserialize<'de> + Default> Deserialize<'de> for GoSlice<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut out = Self::default();
        Self::deserialize_in_place(d, &mut out)?;
        Ok(out)
    }
    fn deserialize_in_place<D: serde::Deserializer<'de>>(
        d: D,
        out: &mut Self,
    ) -> Result<(), D::Error> {
        struct Slice<'a, T>(&'a mut GoSlice<T>);
        impl<'de, T: Deserialize<'de> + Default> serde::de::Visitor<'de> for Slice<'_, T> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an SDK slice or null")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
                *self.0 = GoSlice::default();
                Ok(())
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                let mut len = 0;
                loop {
                    if len == self.0.slots.len() {
                        self.0.slots.push(T::default());
                    }
                    if seq
                        .next_element_seed(ElementSeed(&mut self.0.slots[len]))?
                        .is_none()
                    {
                        break;
                    }
                    len += 1;
                }
                if len == 0 {
                    self.0.slots.clear();
                }
                self.0.len = Some(len);
                Ok(())
            }
        }
        d.deserialize_any(Slice(out))
    }
}

pub(crate) struct GoPointer<T> {
    fields: Option<String>,
    marker: std::marker::PhantomData<T>,
}
impl<T: serde::Serialize + serde::de::DeserializeOwned> GoPointer<T> {
    pub(crate) fn from_value(value: Option<T>) -> Result<Self, serde_json::Error> {
        let fields = value
            .map(|value| serde_json::to_string(&value))
            .transpose()?;
        Ok(Self {
            fields,
            marker: std::marker::PhantomData,
        })
    }
    pub(crate) fn into_value(self) -> Result<Option<T>, serde_json::Error> {
        self.fields
            .map(|fields| serde_json::from_str(&fields))
            .transpose()
    }
}
pub(crate) struct PointerSeed<'a, T>(pub(crate) &'a mut GoPointer<T>);
impl<'de, T: serde::de::DeserializeOwned> serde::de::DeserializeSeed<'de> for PointerSeed<'_, T> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        let raw = Option::<Box<serde_json::value::RawValue>>::deserialize(d)?;
        let Some(raw) = raw else {
            self.0.fields = None;
            return Ok(());
        };
        let text = raw.get();
        // Validate before a later null can discard an invalid pointer object.
        let _: T = serde_json::from_str(text).map_err(serde::de::Error::custom)?;
        if let Some(fields) = &mut self.0.fields {
            // Decode repeated objects together so slice backing slots survive.
            let inner = text[1..text.len() - 1].trim();
            if !inner.is_empty() {
                fields.pop();
                if !fields[1..].trim().is_empty() {
                    fields.push(',');
                }
                fields.push_str(inner);
                fields.push('}');
            }
        } else {
            self.0.fields = Some(text.to_owned());
        }
        Ok(())
    }
}

pub(crate) struct MapSeed<'a, T>(pub(crate) &'a mut Option<std::collections::BTreeMap<String, T>>);
impl<'de, T: Deserialize<'de>> serde::de::DeserializeSeed<'de> for MapSeed<'_, T> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_option(self)
    }
}
impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for MapSeed<'_, T> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an SDK map or null")
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        *self.0 = None;
        Ok(())
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_map(self)
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let values = self.0.get_or_insert_with(Default::default);
        while let Some((key, value)) = map.next_entry()? {
            values.insert(key, value);
        }
        Ok(())
    }
}

macro_rules! read_field {
    ($map:ident, $out:expr, pointer) => {
        $map.next_value_seed($crate::go_decode::PointerSeed(&mut $out))?;
    };
    ($map:ident, $out:expr, map) => {
        $map.next_value_seed($crate::go_decode::MapSeed(&mut $out))?;
    };
    ($map:ident, $out:expr, integer) => {
        $map.next_value_seed($crate::go_decode::IntegerSeed(&mut $out))?;
    };
    ($map:ident, $out:expr, scalar) => {
        $map.next_value_seed(ElementSeed(&mut $out))?;
    };
    ($map:ident, $out:expr, raw) => {
        $out = $map.next_value()?;
    };
    ($map:ident, $out:expr, slice) => {
        struct SliceSeed<'a, T>(&'a mut GoSlice<T>);
        impl<'de, T: Deserialize<'de> + Default> serde::de::DeserializeSeed<'de> for SliceSeed<'_, T> {
            type Value = ();
            fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
                GoSlice::deserialize_in_place(d, self.0)
            }
        }
        $map.next_value_seed(SliceSeed(&mut $out))?;
    };
}

macro_rules! go_object {
    ($ty:ident, $( $key:literal => $field:ident : $kind:ident ),* $(,)? $(; $( $slice_key:literal => $slice:ident ),* )? $( preserve_nil { $( $nil_key:literal => $nil_slice:ident ),* } )? $( pointers { $( $ptr_key:literal => $ptr:ident ),* } )?) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let mut out = Self::default();
                Self::deserialize_in_place(d, &mut out)?;
                Ok(out)
            }
            fn deserialize_in_place<D: serde::Deserializer<'de>>(d: D, out: &mut Self) -> Result<(), D::Error> {
                struct Object<'a>(&'a mut $ty);
                impl<'de> serde::de::Visitor<'de> for Object<'_> {
                    type Value = ();
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str("an SDK input object")
                    }
                    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> { Ok(()) }
                    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
                        d.deserialize_map(self)
                    }
                    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
                        let out = self.0;
                        $($(let mut $slice = GoSlice::from_values(std::mem::take(&mut out.$slice));)*)?
                        $($(let mut $nil_slice = out.$nil_slice.take().map(GoSlice::from_values).unwrap_or_default();)*)?
                        $($(let mut $ptr = $crate::go_decode::GoPointer::from_value(out.$ptr.take()).map_err(serde::de::Error::custom)?;)*)?
                        while let Some(key) = map.next_key::<String>()? {
                            let key = fold_key(&key);
                            match key.as_str() {
                                $( $key => { read_field!(map, out.$field, $kind); } )*
                                $($($slice_key => { read_field!(map, $slice, slice); })*)?
                                $($($nil_key => { read_field!(map, $nil_slice, slice); })*)?
                                $($($ptr_key => { read_field!(map, $ptr, pointer); })*)?
                                _ => { map.next_value::<serde::de::IgnoredAny>()?; }
                            }
                        }
                        $($(out.$slice = $slice.into_values().unwrap_or_default();)*)?
                        $($(out.$nil_slice = $nil_slice.into_values();)*)?
                        $($(out.$ptr = $ptr.into_value().map_err(serde::de::Error::custom)?;)*)?
                        Ok(())
                    }
                }
                d.deserialize_option(Object(out))
            }
        }
    };
}

pub(crate) use {go_object, read_field};

pub(crate) fn fold_key(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            'K' => 'k',
            'ſ' => 's',
            _ => c.to_ascii_lowercase(),
        })
        .collect()
}

pub(crate) fn go_utf8(mut input: &[u8]) -> String {
    let mut text = String::new();
    while !input.is_empty() {
        match std::str::from_utf8(input) {
            Ok(rest) => {
                text.push_str(rest);
                break;
            }
            Err(error) => {
                let (valid, rest) = input.split_at(error.valid_up_to());
                text.push_str(std::str::from_utf8(valid).expect("validated prefix"));
                let invalid = error.error_len().unwrap_or(rest.len());
                text.extend(std::iter::repeat_n('\u{fffd}', invalid));
                input = &rest[invalid..];
            }
        }
    }
    text
}

pub(crate) fn normalize_strings(input: &[u8]) -> Vec<u8> {
    let mut bytes = go_utf8(input).into_bytes();
    let mut inside = false;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            inside = !inside;
        } else if inside && bytes[i] == b'\\' {
            if bytes.get(i + 1) == Some(&b'u') {
                let hex = |start: usize| {
                    bytes
                        .get(start..start + 4)
                        .and_then(|digits| std::str::from_utf8(digits).ok())
                        .and_then(|digits| u16::from_str_radix(digits, 16).ok())
                };
                if let Some(code) = hex(i + 2) {
                    if (0xd800..=0xdbff).contains(&code)
                        && bytes.get(i + 6..i + 8) == Some(b"\\u")
                        && hex(i + 8).is_some_and(|low| (0xdc00..=0xdfff).contains(&low))
                    {
                        i += 12;
                        continue;
                    }
                    // encoding/json replaces unpaired surrogates instead of rejecting the string.
                    if (0xd800..=0xdfff).contains(&code) {
                        bytes[i + 2..i + 6].copy_from_slice(b"fffd");
                    }
                    i += 6;
                    continue;
                }
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    bytes
}
