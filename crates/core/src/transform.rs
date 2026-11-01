//! Text transforms offered when pasting (Shift+Enter menu in the popup). Each is a pure function of the text.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform {
    Trim,
    Upper,
    Lower,
    /// Joins the lines with single spaces (for a copied paragraph that was hard-wrapped).
    OneLine,
}

impl Transform {
    /// The position in the popup's options menu (after its "Original" and "Plain text" entries).
    pub fn from_index(i: i32) -> Option<Self> {
        match i {
            0 => Some(Self::Trim),
            1 => Some(Self::Upper),
            2 => Some(Self::Lower),
            3 => Some(Self::OneLine),
            _ => None,
        }
    }

    pub fn apply(self, text: &str) -> String {
        match self {
            Self::Trim => text.trim().to_string(),
            Self::Upper => text.to_uppercase(),
            Self::Lower => text.to_lowercase(),
            Self::OneLine => text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Transform::*;

    #[test]
    fn transforms() {
        assert_eq!(Trim.apply("  a b \n"), "a b");
        assert_eq!(Upper.apply("straße ok"), "STRASSE OK");
        assert_eq!(Lower.apply("ÁBC"), "ábc");
        assert_eq!(
            OneLine.apply("  line one\n\n   line two  \r\nthree"),
            "line one line two three"
        );
        assert_eq!(super::Transform::from_index(3), Some(OneLine));
        assert_eq!(super::Transform::from_index(4), None);
    }
}
