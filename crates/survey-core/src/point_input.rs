//! Typed point entry: absolute, relative (`@dE,dN`) and polar (`@dist<bearing`).
//!
//! Absolute and relative coordinates use the active [`Conventions`] coordinate
//! order and units; the bearing in polar input uses its angle conventions.

use glam::DVec3;

use crate::angle::Bearing;
use crate::conventions::{Conventions, split_values};
use crate::error::ParseError;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointInput {
    Absolute { e: f64, n: f64, z: Option<f64> },
    Relative { de: f64, dn: f64, dz: Option<f64> },
    Polar { distance: f64, bearing: Bearing },
}

/// A resolved point plus whether its height is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedPoint {
    pub position: DVec3,
    pub height_known: bool,
}

impl PointInput {
    /// Returns `Ok(None)` when the text does not look like point input at all
    /// (so callers can treat it as a command or option instead).
    pub fn parse(text: &str, conv: &Conventions) -> Result<Option<PointInput>, ParseError> {
        let t = text.trim();
        if let Some(rel) = t.strip_prefix('@') {
            if let Some((dist, brg)) = rel.split_once('<') {
                return Ok(Some(PointInput::Polar {
                    distance: conv.parse_length(dist)?,
                    bearing: conv.parse_bearing(brg)?,
                }));
            }
            let c = conv.parse_coord(rel)?;
            return Ok(Some(PointInput::Relative {
                de: c.easting,
                dn: c.northing,
                dz: c.height,
            }));
        }
        let looks_numeric = t
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == '+');
        if !looks_numeric || split_values(t).len() < 2 {
            return Ok(None);
        }
        let c = conv.parse_coord(t)?;
        Ok(Some(PointInput::Absolute {
            e: c.easting,
            n: c.northing,
            z: c.height,
        }))
    }

    /// Resolves against an optional base point (required for relative/polar input).
    pub fn resolve(self, base: Option<ResolvedPoint>) -> Result<ResolvedPoint, ParseError> {
        match self {
            PointInput::Absolute { e, n, z } => Ok(ResolvedPoint {
                position: DVec3::new(e, n, z.unwrap_or(0.0)),
                height_known: z.is_some(),
            }),
            PointInput::Relative { de, dn, dz } => {
                let base = base.ok_or_else(no_base)?;
                let height_known = base.height_known || dz.is_some();
                Ok(ResolvedPoint {
                    position: base.position + DVec3::new(de, dn, dz.unwrap_or(0.0)),
                    height_known,
                })
            }
            PointInput::Polar { distance, bearing } => {
                let base = base.ok_or_else(no_base)?;
                let d = bearing.unit_vector() * distance;
                Ok(ResolvedPoint {
                    position: base.position + d.extend(0.0),
                    height_known: base.height_known,
                })
            }
        }
    }
}

fn no_base() -> ParseError {
    ParseError::Invalid("relative input needs a previous point".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conventions::CoordOrder;

    fn base() -> Option<ResolvedPoint> {
        Some(ResolvedPoint {
            position: DVec3::new(500_000.0, 7_000_000.0, 100.0),
            height_known: true,
        })
    }

    #[test]
    fn absolute_relative_polar() {
        let c = Conventions::metric_generic();
        let p = PointInput::parse("1000,2000", &c).expect("ok").expect("point");
        let r = p.resolve(None).expect("abs");
        assert_eq!(r.position, DVec3::new(1000.0, 2000.0, 0.0));
        assert!(!r.height_known);

        let p = PointInput::parse("@10,-5,1", &c).expect("ok").expect("point");
        let r = p.resolve(base()).expect("rel");
        assert_eq!(r.position, DVec3::new(500_010.0, 6_999_995.0, 101.0));

        let p = PointInput::parse("@100<90", &c).expect("ok").expect("point");
        let r = p.resolve(base()).expect("polar");
        assert!((r.position - DVec3::new(500_100.0, 7_000_000.0, 100.0)).length() < 1e-9);
        assert!(
            PointInput::parse("@1,1", &c)
                .expect("ok")
                .expect("p")
                .resolve(None)
                .is_err()
        );
    }

    #[test]
    fn respects_coordinate_order() {
        let mut c = Conventions::metric_generic();
        c.coord_order = CoordOrder::NorthingEasting;
        let p = PointInput::parse("7000000 500000 5", &c).expect("ok").expect("p");
        let r = p.resolve(None).expect("abs");
        assert_eq!(r.position, DVec3::new(500_000.0, 7_000_000.0, 5.0));
    }

    #[test]
    fn non_point_text() {
        let c = Conventions::metric_generic();
        assert_eq!(PointInput::parse("close", &c), Ok(None));
        assert_eq!(PointInput::parse("12", &c), Ok(None));
        assert!(PointInput::parse("1,abc", &c).is_err());
    }
}
