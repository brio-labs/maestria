//! Macro for generating mechanical bidirectional enum-mirror conversions
//! between storage DTOs and domain enums (Rule 37 DTO boundary).

#[macro_export]
macro_rules! stored_enum {
    (
        $(#[$meta:meta])*
        pub(crate) enum $stored:ident <=> $domain:ident {
            $(
                $(#[$vmeta:meta])*
                $variant:ident
            ),+ $(,)?
        }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        $(#[$meta])*
        pub(crate) enum $stored {
            $(
                $(#[$vmeta])*
                $variant,
            )+
        }

        impl $stored {
            pub(crate) fn from_domain(value: impl std::borrow::Borrow<$domain>) -> Self {
                match value.borrow() {
                    $($domain::$variant => Self::$variant,)+
                }
            }

            pub(crate) fn try_into_domain(self) -> Result<$domain, maestria_ports::PortError> {
                Ok(match self {
                    $(Self::$variant => $domain::$variant,)+
                })
            }
        }

        impl From<$domain> for $stored {
            fn from(value: $domain) -> Self {
                Self::from_domain(value)
            }
        }

        impl From<$stored> for $domain {
            fn from(value: $stored) -> Self {
                match value {
                    $($stored::$variant => $domain::$variant,)+
                }
            }
        }
    };
}
