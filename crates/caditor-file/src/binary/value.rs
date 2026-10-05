use std::fmt::Display;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{
        self, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
        Visitor,
    },
    ser::{
        self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
        SerializeTupleStruct, SerializeTupleVariant,
    },
};

const NULL: u8 = 0x00;
const FALSE: u8 = 0x01;
const TRUE: u8 = 0x02;
const UNSIGNED: u8 = 0x03;
const NEGATIVE: u8 = 0x04;
const FLOAT: u8 = 0x05;
const STRING: u8 = 0x06;
const BYTES: u8 = 0x07;
const SEQUENCE: u8 = 0x08;
const MAP: u8 = 0x09;
const END: u8 = 0x0a;
const MAX_DEPTH: usize = 128;
const VARINT_BITS: u32 = 7;
const VARINT_CONTINUES: u8 = 0x80;
const VARINT_PAYLOAD: u8 = 0x7f;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Damage {
    #[error("the value ends early")]
    EndsEarly,
    #[error("a number is damaged")]
    DamagedNumber,
    #[error("a number ends early")]
    NumberEndsEarly,
    #[error("a number is too small")]
    NumberTooSmall,
    #[error("a length is too large")]
    LengthTooLarge,
    #[error("a length runs past the end")]
    LengthPastEnd,
    #[error("a text is not valid UTF-8")]
    InvalidText,
    #[error("the value is nested too deeply")]
    NestedTooDeeply,
    #[error("a list or map holds more than expected")]
    TooManyElements,
    #[error("expected a variant")]
    ExpectedVariant,
    #[error("unknown value kind")]
    UnknownKind,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub(crate) enum ValueError {
    #[error("{damage} at byte {position}")]
    Malformed { damage: Damage, position: usize },
    #[error("the value is followed by unexpected bytes")]
    TrailingBytes,
    #[error("a value is {0}, which cannot be stored; only finite numbers can")]
    NonFinite(f64),
    #[error("a record lacks the field “{field}”")]
    MissingField { field: &'static str },
    #[error("a record holds the field “{field}” twice")]
    DuplicateField { field: &'static str },
    #[error("a record holds the unknown field “{field}”")]
    UnknownField { field: String },
    #[error("“{variant}” is not one of the variants a value may take")]
    UnknownVariant { variant: String },
    #[error("a value holds {found} entries where {expected} were expected")]
    WrongLength { found: usize, expected: String },
    #[error("a value is {found} where {expected} was expected")]
    WrongType { found: String, expected: String },
    #[error("a type refused its stored value")]
    Refused,
}

impl ser::Error for ValueError {
    fn custom<T: Display>(_message: T) -> Self {
        Self::Refused
    }
}

impl de::Error for ValueError {
    fn custom<T: Display>(_message: T) -> Self {
        Self::Refused
    }

    fn invalid_type(found: de::Unexpected<'_>, expected: &dyn de::Expected) -> Self {
        Self::WrongType {
            found: found.to_string(),
            expected: expected.to_string(),
        }
    }

    fn invalid_value(found: de::Unexpected<'_>, expected: &dyn de::Expected) -> Self {
        Self::WrongType {
            found: found.to_string(),
            expected: expected.to_string(),
        }
    }

    fn invalid_length(found: usize, expected: &dyn de::Expected) -> Self {
        Self::WrongLength {
            found,
            expected: expected.to_string(),
        }
    }

    fn unknown_variant(variant: &str, _expected: &'static [&'static str]) -> Self {
        Self::UnknownVariant {
            variant: variant.to_owned(),
        }
    }

    fn unknown_field(field: &str, _expected: &'static [&'static str]) -> Self {
        Self::UnknownField {
            field: field.to_owned(),
        }
    }

    fn missing_field(field: &'static str) -> Self {
        Self::MissingField { field }
    }

    fn duplicate_field(field: &'static str) -> Self {
        Self::DuplicateField { field }
    }
}

pub(crate) fn to_bytes(value: &impl Serialize) -> Result<Vec<u8>, ValueError> {
    let mut encoder = Encoder { bytes: Vec::new() };
    value.serialize(&mut encoder)?;
    Ok(encoder.bytes)
}

pub(crate) fn from_bytes<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, ValueError> {
    let mut decoder = Decoder {
        bytes,
        position: 0,
        depth: 0,
    };
    let value = T::deserialize(&mut decoder)?;
    if decoder.position != bytes.len() {
        return Err(ValueError::TrailingBytes);
    }
    Ok(value)
}

pub(crate) fn push_varint(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let low = (value & u64::from(VARINT_PAYLOAD)) as u8;
        value >>= VARINT_BITS;
        if value == 0 {
            bytes.push(low);
            return;
        }
        bytes.push(low | VARINT_CONTINUES);
    }
}

pub(crate) fn read_varint(bytes: &[u8], position: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    let mut shift = 0_u32;
    loop {
        let byte = *bytes.get(*position)?;
        *position += 1;
        let payload = u64::from(byte & VARINT_PAYLOAD);
        if shift >= u64::BITS || (shift > 0 && payload >> (u64::BITS - shift) != 0) {
            return None;
        }
        value |= payload << shift;
        if byte & VARINT_CONTINUES == 0 {
            return Some(value);
        }
        shift += VARINT_BITS;
    }
}

struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    fn tag(&mut self, tag: u8) {
        self.bytes.push(tag);
    }

    fn unsigned(&mut self, value: u64) {
        self.tag(UNSIGNED);
        push_varint(&mut self.bytes, value);
    }

    fn signed(&mut self, value: i64) {
        match u64::try_from(value) {
            Ok(value) => self.unsigned(value),
            Err(_) => {
                self.tag(NEGATIVE);
                push_varint(&mut self.bytes, !(value as u64));
            }
        }
    }

    fn string(&mut self, value: &str) {
        self.tag(STRING);
        push_varint(&mut self.bytes, value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
    }
}

impl Serializer for &mut Encoder {
    type Ok = ();
    type Error = ValueError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, value: bool) -> Result<(), ValueError> {
        self.tag(if value { TRUE } else { FALSE });
        Ok(())
    }

    fn serialize_i8(self, value: i8) -> Result<(), ValueError> {
        self.signed(value.into());
        Ok(())
    }

    fn serialize_i16(self, value: i16) -> Result<(), ValueError> {
        self.signed(value.into());
        Ok(())
    }

    fn serialize_i32(self, value: i32) -> Result<(), ValueError> {
        self.signed(value.into());
        Ok(())
    }

    fn serialize_i64(self, value: i64) -> Result<(), ValueError> {
        self.signed(value);
        Ok(())
    }

    fn serialize_u8(self, value: u8) -> Result<(), ValueError> {
        self.unsigned(value.into());
        Ok(())
    }

    fn serialize_u16(self, value: u16) -> Result<(), ValueError> {
        self.unsigned(value.into());
        Ok(())
    }

    fn serialize_u32(self, value: u32) -> Result<(), ValueError> {
        self.unsigned(value.into());
        Ok(())
    }

    fn serialize_u64(self, value: u64) -> Result<(), ValueError> {
        self.unsigned(value);
        Ok(())
    }

    fn serialize_f32(self, value: f32) -> Result<(), ValueError> {
        self.serialize_f64(value.into())
    }

    fn serialize_f64(self, value: f64) -> Result<(), ValueError> {
        if !value.is_finite() {
            return Err(ValueError::NonFinite(value));
        }
        self.tag(FLOAT);
        self.bytes.extend_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn serialize_char(self, value: char) -> Result<(), ValueError> {
        self.string(value.encode_utf8(&mut [0; 4]));
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result<(), ValueError> {
        self.string(value);
        Ok(())
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), ValueError> {
        self.tag(BYTES);
        push_varint(&mut self.bytes, value.len() as u64);
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn serialize_none(self) -> Result<(), ValueError> {
        self.serialize_unit()
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), ValueError> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), ValueError> {
        self.tag(NULL);
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), ValueError> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<(), ValueError> {
        self.string(variant);
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), ValueError> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), ValueError> {
        self.tag(MAP);
        self.string(variant);
        value.serialize(&mut *self)?;
        self.tag(END);
        Ok(())
    }

    fn serialize_seq(self, _length: Option<usize>) -> Result<Self, ValueError> {
        self.tag(SEQUENCE);
        Ok(self)
    }

    fn serialize_tuple(self, _length: usize) -> Result<Self, ValueError> {
        self.serialize_seq(None)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Self, ValueError> {
        self.serialize_seq(None)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _length: usize,
    ) -> Result<Self, ValueError> {
        self.tag(MAP);
        self.string(variant);
        self.serialize_seq(None)
    }

    fn serialize_map(self, _length: Option<usize>) -> Result<Self, ValueError> {
        self.tag(MAP);
        Ok(self)
    }

    fn serialize_struct(self, _name: &'static str, _length: usize) -> Result<Self, ValueError> {
        self.serialize_map(None)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _length: usize,
    ) -> Result<Self, ValueError> {
        self.tag(MAP);
        self.string(variant);
        self.serialize_map(None)
    }
}

impl SerializeSeq for &mut Encoder {
    type Ok = ();
    type Error = ValueError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), ValueError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ValueError> {
        self.tag(END);
        Ok(())
    }
}

impl SerializeTuple for &mut Encoder {
    type Ok = ();
    type Error = ValueError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), ValueError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ValueError> {
        self.tag(END);
        Ok(())
    }
}

impl SerializeTupleStruct for &mut Encoder {
    type Ok = ();
    type Error = ValueError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), ValueError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ValueError> {
        self.tag(END);
        Ok(())
    }
}

impl SerializeTupleVariant for &mut Encoder {
    type Ok = ();
    type Error = ValueError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), ValueError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ValueError> {
        self.tag(END);
        self.tag(END);
        Ok(())
    }
}

impl SerializeMap for &mut Encoder {
    type Ok = ();
    type Error = ValueError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), ValueError> {
        key.serialize(&mut **self)
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), ValueError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ValueError> {
        self.tag(END);
        Ok(())
    }
}

impl SerializeStruct for &mut Encoder {
    type Ok = ();
    type Error = ValueError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), ValueError> {
        self.string(key);
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ValueError> {
        self.tag(END);
        Ok(())
    }
}

impl SerializeStructVariant for &mut Encoder {
    type Ok = ();
    type Error = ValueError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), ValueError> {
        self.string(key);
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), ValueError> {
        self.tag(END);
        self.tag(END);
        Ok(())
    }
}

struct Decoder<'de> {
    bytes: &'de [u8],
    position: usize,
    depth: usize,
}

impl<'de> Decoder<'de> {
    fn error(&self, damage: Damage) -> ValueError {
        ValueError::Malformed {
            damage,
            position: self.position,
        }
    }

    fn peek(&self) -> Result<u8, ValueError> {
        self.bytes
            .get(self.position)
            .copied()
            .ok_or_else(|| self.error(Damage::EndsEarly))
    }

    fn next(&mut self) -> Result<u8, ValueError> {
        let byte = self.peek()?;
        self.position += 1;
        Ok(byte)
    }

    fn varint(&mut self) -> Result<u64, ValueError> {
        read_varint(self.bytes, &mut self.position).ok_or_else(|| self.error(Damage::DamagedNumber))
    }

    fn slice(&mut self) -> Result<&'de [u8], ValueError> {
        let length =
            usize::try_from(self.varint()?).map_err(|_| self.error(Damage::LengthTooLarge))?;
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| self.error(Damage::LengthPastEnd))?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| self.error(Damage::LengthPastEnd))?;
        self.position = end;
        Ok(slice)
    }

    fn string(&mut self) -> Result<&'de str, ValueError> {
        let bytes = self.slice()?;
        std::str::from_utf8(bytes).map_err(|_| self.error(Damage::InvalidText))
    }

    fn enter(&mut self) -> Result<(), ValueError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error(Damage::NestedTooDeeply));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn at_end(&mut self) -> Result<bool, ValueError> {
        if self.peek()? == END {
            self.position += 1;
            return Ok(true);
        }
        Ok(false)
    }

    fn expect_end(&mut self) -> Result<(), ValueError> {
        if self.at_end()? {
            Ok(())
        } else {
            Err(self.error(Damage::TooManyElements))
        }
    }
}

impl<'de> Deserializer<'de> for &mut Decoder<'de> {
    type Error = ValueError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ValueError> {
        match self.next()? {
            NULL => visitor.visit_unit(),
            FALSE => visitor.visit_bool(false),
            TRUE => visitor.visit_bool(true),
            UNSIGNED => visitor.visit_u64(self.varint()?),
            NEGATIVE => {
                let magnitude = self.varint()?;
                let value =
                    i64::try_from(magnitude).map_err(|_| self.error(Damage::NumberTooSmall))?;
                visitor.visit_i64(!value)
            }
            FLOAT => {
                let end = self.position + 8;
                let bytes: [u8; 8] = self
                    .bytes
                    .get(self.position..end)
                    .and_then(|bytes| bytes.try_into().ok())
                    .ok_or_else(|| self.error(Damage::NumberEndsEarly))?;
                self.position = end;
                visitor.visit_f64(f64::from_le_bytes(bytes))
            }
            STRING => visitor.visit_borrowed_str(self.string()?),
            BYTES => visitor.visit_borrowed_bytes(self.slice()?),
            SEQUENCE => {
                self.enter()?;
                let mut elements = Elements {
                    decoder: self,
                    finished: false,
                };
                let value = visitor.visit_seq(&mut elements)?;
                if !elements.finished {
                    self.expect_end()?;
                }
                self.leave();
                Ok(value)
            }
            MAP => {
                self.enter()?;
                let mut entries = Entries {
                    decoder: self,
                    finished: false,
                };
                let value = visitor.visit_map(&mut entries)?;
                if !entries.finished {
                    self.expect_end()?;
                }
                self.leave();
                Ok(value)
            }
            _ => Err(ValueError::Malformed {
                damage: Damage::UnknownKind,
                position: self.position - 1,
            }),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ValueError> {
        if self.peek()? == NULL {
            self.position += 1;
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ValueError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ValueError> {
        match self.next()? {
            STRING => {
                let variant = self.string()?;
                visitor.visit_enum(variant.into_deserializer())
            }
            MAP => {
                self.enter()?;
                let value = visitor.visit_enum(Variant { decoder: self })?;
                self.expect_end()?;
                self.leave();
                Ok(value)
            }
            _ => Err(self.error(Damage::ExpectedVariant)),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

struct Elements<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
    finished: bool,
}

impl<'de> SeqAccess<'de> for &mut Elements<'_, 'de> {
    type Error = ValueError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, ValueError> {
        if self.finished {
            return Ok(None);
        }
        if self.decoder.at_end()? {
            self.finished = true;
            return Ok(None);
        }
        seed.deserialize(&mut *self.decoder).map(Some)
    }
}

struct Entries<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
    finished: bool,
}

impl<'de> MapAccess<'de> for &mut Entries<'_, 'de> {
    type Error = ValueError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ValueError> {
        if self.finished {
            return Ok(None);
        }
        if self.decoder.at_end()? {
            self.finished = true;
            return Ok(None);
        }
        seed.deserialize(&mut *self.decoder).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, ValueError> {
        seed.deserialize(&mut *self.decoder)
    }
}

struct Variant<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
}

impl<'de> EnumAccess<'de> for Variant<'_, 'de> {
    type Error = ValueError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self), ValueError> {
        let variant = seed.deserialize(&mut *self.decoder)?;
        Ok((variant, self))
    }
}

impl<'de> VariantAccess<'de> for Variant<'_, 'de> {
    type Error = ValueError;

    fn unit_variant(self) -> Result<(), ValueError> {
        de::IgnoredAny::deserialize(&mut *self.decoder).map(drop)
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, ValueError> {
        seed.deserialize(&mut *self.decoder)
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _length: usize,
        visitor: V,
    ) -> Result<V::Value, ValueError> {
        de::Deserializer::deserialize_any(&mut *self.decoder, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ValueError> {
        de::Deserializer::deserialize_any(&mut *self.decoder, visitor)
    }
}
