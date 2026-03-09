use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutKind {
    #[default]
    LinkedWorktree,
    DiscreteClone,
}

impl CheckoutKind {
    pub fn action_label(self) -> &'static str {
        match self {
            Self::LinkedWorktree => "Create Worktree",
            Self::DiscreteClone => "Create Discrete Clone",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LinkedWorktree => "Worktree",
            Self::DiscreteClone => "Discrete Clone",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::LinkedWorktree => "Shares the repository storage and creates a linked checkout.",
            Self::DiscreteClone => {
                "Creates a separate checkout with its own .git directory and remote config."
            },
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::LinkedWorktree => "\u{e725}",
            Self::DiscreteClone => "\u{f0c5}",
        }
    }
}
