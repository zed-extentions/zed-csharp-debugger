//! JSONC (JSON with comments and trailing commas) parsing for `.vscode/launch.json` and
//! `launchSettings.json`, delegated to the `jsonc-parser` crate (MIT, pure Rust).
//!
//! Only the two JSONC extensions VS Code and Visual Studio accept in these files are enabled;
//! everything else stays strict JSON so malformed input still produces an error.

use crate::serde_json::Value;
use jsonc_parser::{parse_to_serde_value, ParseOptions};

const OPTIONS: ParseOptions = ParseOptions {
    allow_comments: true,
    allow_trailing_commas: true,
    allow_loose_object_property_names: false,
    allow_missing_commas: false,
    allow_single_quoted_strings: false,
    allow_hexadecimal_numbers: false,
    allow_unary_plus_numbers: false,
    allow_bare_decimal_point_numbers: false,
    allow_non_finite_numbers: false,
    allow_extended_string_escapes: false,
};

/// Parses JSONC text into a JSON value. Empty or whitespace-only input yields `Value::Null`.
pub fn parse_jsonc(input: &str) -> Result<Value, String> {
    parse_to_serde_value(input, &OPTIONS).map_err(|error| error.to_string())
}
