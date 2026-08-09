use std::fmt;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde::de::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSize(i64); // DB is an i64, matching type here

// Units for Convenince
pub const BYTE:     i64 = 1;
pub const KILOBYTE: i64 = 1000;
pub const KIBIBYTE: i64 = 1024;
pub const MEGABYTE: i64 = KILOBYTE * 1000;
pub const MEBIBYTE: i64 = KIBIBYTE * 1024;
pub const GIGABYTE: i64 = MEGABYTE * 1000;
pub const GIBIBYTE: i64 = MEBIBYTE * 1024;
pub const TERABYTE: i64 = GIGABYTE * 1000;
pub const TEBIBYTE: i64 = GIBIBYTE * 1024;
pub const PETABYTE: i64 = TERABYTE * 1000;
pub const PEPIBYTE: i64 = TEBIBYTE * 1024;
pub const EXABYTE:  i64 = PETABYTE * 1000;
pub const EXBIBYTE: i64 = PEPIBYTE * 1024;

/// Conversion for units (Uppercase matcher, unit affix, bytes)
///
/// Largest first, so serialization takes the largest unit that divides evenly.
/// B sits last and catches any count the units above it leave a remainder on
const UNITS: [(&str, &str, i64); 13] = [
    ("EIB", "EiB", EXBIBYTE),
    ("EB",  "EB",  EXABYTE),
    ("PIB", "PiB", PEPIBYTE),
    ("PB",  "PB",  PETABYTE),
    ("TIB", "TiB", TEBIBYTE),
    ("TB",  "TB",  TERABYTE),
    ("GIB", "GiB", GIBIBYTE),
    ("GB",  "GB",  GIGABYTE),
    ("MIB", "MiB", MEBIBYTE),
    ("MB",  "MB",  MEGABYTE),
    ("KIB", "KiB", KIBIBYTE),
    ("KB",  "KB",  KILOBYTE),
    ("B",   "B",   BYTE),
];

impl ByteSize {

    pub const fn from_int(input: i64) -> Self {
        ByteSize(input)
    }

    pub const fn to_int(self) -> i64 {
        self.0
    }

    pub fn parse(input: &str) -> Result<Self, String> {

        // Trim input string to match format (###UNIT) where # is a digit and UNIT is an affix like "MB"
        let cleaned = input.split_whitespace().collect::<String>().to_ascii_uppercase();

        // Bytes cannot be negative
        if cleaned.starts_with('-') {
            return Err(format!("{input} is negative"));
        }
        if cleaned.contains('.') {
            return Err(format!("{input} cannot contain decimal numbers, only integers"))
        }

        // Start at end of string (no unit mean we assume bytes later)
        let mut split_index = cleaned.len();

        // Extract value and unit from a string like 25MB into "25" and "MB"
        for (index, character) in cleaned.char_indices() {
            if !character.is_ascii_digit() {
                split_index = index; // Move index back to first ASCII character
                break;
            }
        }

        // Set split values into their respective vars
        let digits = &cleaned[0..split_index];
        let parsed_unit = &cleaned[split_index..];

        // Attempt to convert the value to an integer
        let value = match digits.parse::<i64>() {
            Ok(value) => value,
            Err(_) => return Err(format!("{input} is not a number followed by a unit"))
        };

        // Is no unit is found assume bytes
        let parsed_unit = if parsed_unit.is_empty() {
            UNITS[UNITS.len()-1].0 // "B"
        } else {parsed_unit};

        // Multiply read value by the unit value
        for (upper, _, bytes) in UNITS {
            if parsed_unit == upper {
                // Plain multiplication wraps in a release build, turning an
                // oversized value into a small one that passes every later check
                return match value.checked_mul(bytes) {
                    Some(bytes) => Ok(ByteSize(bytes)),
                    None => Err(format!("{input} is larger than i64 holds"))
                };
            }
        }

        // Reaching here means the unit matched nothing in the table
        Err(format!("{parsed_unit} is not a known unit"))
    }
}

impl fmt::Display for ByteSize {

    fn fmt (&self, f: &mut fmt::Formatter) -> fmt::Result {

        // Convert i64 into a readable string (default to Bytes)
        for (_, unit, bytes) in UNITS {
            if (self.0 >= bytes) && (self.0 % bytes == 0) {
                return write!(f, "{}{}", self.0 / bytes, unit);
            }
        }

        write!(f, "{}B", self.0) // 0 Bytes
    }
}

impl Serialize for ByteSize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Both forms config file can hold:
/// - Quoted value: A value/unit string (ie. 25MB)
/// - Bare integer: Count in bytes (ie. 1000)
#[derive(Deserialize)]
#[serde(untagged)]
enum Raw {
    Text(String),
    Number(i64)
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>
    {
        let raw = Raw::deserialize(deserializer)?;

        match raw {

            // Text route
            Raw::Text(text) => match ByteSize::parse(&text) {
                Ok(val) => Ok(val),
                Err(reason) => Err(D::Error::custom(reason))
            },

            // Integer route
            Raw::Number(bytes) => {
                if bytes < 0 {
                    return Err(D::Error::custom(format!("{bytes} is negative")));
                }
                Ok(ByteSize::from_int(bytes))
            }
        }
    }
}
