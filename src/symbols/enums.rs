//! Enum data types declared in `DataTypes`.
pub type EnumId = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumType {
    pub name: String,
    pub members: Vec<(String, i64)>, // (name, ContainerOrder)
    pub default: Option<String>,
    /// `true` when the member list is *not* exhaustively known — a
    /// firmware-supplied enum (`MoTeC Types.<N>` / `::Hardware.<N>`) whose
    /// members are defined by the firmware, not the project. Membership-based
    /// checks (T020 enum-non-member) must be suppressed for open enums, since a
    /// name absent from `members` is not provably a non-member; type-level
    /// checks (T021 enum-vs-number, enum hover) still apply.
    pub open: bool,
}
