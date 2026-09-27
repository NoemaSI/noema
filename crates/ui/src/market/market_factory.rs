#[derive(Debug, Clone, Copy)]
pub enum MarketVariant {
    Mechanism,
    Law,
    Blueprint,
    Machine,
    Storage,
    Transport,
}

pub struct MarketSection {
    label: String,
    variant: MarketVariant,
}

impl MarketVariant {
    pub const ALL: [Self; 6] = [
        Self::Mechanism,
        Self::Law,
        Self::Blueprint,
        Self::Machine,
        Self::Storage,
        Self::Transport,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mechanism => "Mechanism",
            Self::Law => "Law",
            Self::Blueprint => "Blueprint",
            Self::Machine => "Machine",
            Self::Storage => "Storage",
            Self::Transport => "Transport",
        }
    }
}

pub fn build_market() -> Vec<MarketSection> {
    MarketVariant::ALL
        .into_iter()
        .map(|variant| MarketSection {
            label: variant.label().to_string(),
            variant,
        })
        .collect()
}

