//! Pure surveying mathematics with no UI or I/O dependencies.
//!
//! All values are stored canonically: lengths in metres, angles in radians and
//! coordinates as `(easting, northing, height)` in `f64`. Regional conventions
//! (bearing style, coordinate order, units) only apply when parsing user input
//! or formatting output — see [`conventions::Conventions`].

pub mod accuracy;
pub mod adjustment;
pub mod alignment;
pub mod angle;
pub mod arc;
pub mod calibration;
pub mod closure;
pub mod cogo;
pub mod contour;
pub mod conventions;
pub mod curve_sets;
pub mod error;
pub mod field_to_finish;
pub mod geoid;
pub mod geom;
pub mod georef;
pub mod network;
pub mod offset;
pub mod point_input;
pub mod projection;
pub mod reduction;
pub mod resection;
pub mod solvers;
pub mod spiral;
pub mod style;
pub mod subdivide;
pub mod tin;
pub mod traverse;
pub mod trim;
pub mod volume;

pub use angle::{Angle, Bearing, Dms, Quadrant};
pub use conventions::Conventions;
pub use error::ParseError;
pub use geom::Aabb3;
pub use glam::{DVec2, DVec3};
