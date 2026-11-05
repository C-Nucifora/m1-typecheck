//! M1 Build project-format compatibility. Independent of the intrinsic catalogue.
use std::{fmt, str::FromStr};

/// Compiler baseline used when interpreting function signatures.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BuildVersion {
    /// Stable release, using scalar `Signature ReturnType` declarations.
    Stable144981,
    /// Beta release, also accepting ordered `Returns` declarations.
    #[default]
    Beta145556,
}

impl BuildVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable144981 => "1.4.4.0981",
            Self::Beta145556 => "1.4.5.0556",
        }
    }

    /// Read a supported `ProductVersion`. Missing or unrecognized metadata
    /// retains the default profile; callers can explicitly override it.
    pub fn from_project_xml(xml: &str) -> Self {
        roxmltree::Document::parse(xml)
            .ok()
            .and_then(|doc| doc.root_element().attribute("ProductVersion")?.parse().ok())
            .unwrap_or_default()
    }
}

impl fmt::Display for BuildVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BuildVersion {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "1.4.4.0981" | "1.4.4.981" | "01.04.04.0981" => Ok(Self::Stable144981),
            "1.4.5.0556" | "1.4.5.556" | "01.04.05.0556" => Ok(Self::Beta145556),
            _ => Err(format!(
                "unsupported M1 Build version `{value}`; supported versions: 1.4.4.0981, 1.4.5.0556"
            )),
        }
    }
}
