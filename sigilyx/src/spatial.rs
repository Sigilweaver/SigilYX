//! SHP ↔ WKB geometry conversion for Alteryx YXDB `SpatialObj` fields.
//!
//! Alteryx stores geometry data in `SpatialObj` fields using the ESRI Shapefile
//! (SHP) record format - each cell contains a single SHP geometry record
//! (shape type + coordinate arrays). This module converts between that
//! format and ISO WKB (Well-Known Binary), which is understood by all
//! major GIS libraries (Shapely, GeoPandas, PostGIS, GDAL, etc.).
//!
//! # Supported geometry types
//!
//! | SHP type          | WKB type             |
//! |--------------------|----------------------|
//! | Null (0)           | empty / null         |
//! | Point (1)          | Point                |
//! | PolyLine (3)       | MultiLineString      |
//! | Polygon (5)        | MultiPolygon         |
//! | MultiPoint (8)     | MultiPoint           |
//! | PointZ (11)        | Point Z / Point ZM   |
//! | PolyLineZ (13)     | MultiLineString Z/ZM |
//! | PolygonZ (15)      | MultiPolygon Z/ZM    |
//! | MultiPointZ (18)   | MultiPoint Z/ZM      |
//! | PointM (21)        | Point M              |
//! | PolyLineM (23)     | MultiLineString M    |
//! | PolygonM (25)      | MultiPolygon M       |
//! | MultiPointM (28)   | MultiPoint M         |

use crate::error::{Result, YxdbError};

// -- SHP Shape Types --

const SHP_NULL: i32 = 0;
const SHP_POINT: i32 = 1;
const SHP_POLYLINE: i32 = 3;
const SHP_POLYGON: i32 = 5;
const SHP_MULTIPOINT: i32 = 8;
const SHP_POINT_Z: i32 = 11;
const SHP_POLYLINE_Z: i32 = 13;
const SHP_POLYGON_Z: i32 = 15;
const SHP_MULTIPOINT_Z: i32 = 18;
const SHP_POINT_M: i32 = 21;
const SHP_POLYLINE_M: i32 = 23;
const SHP_POLYGON_M: i32 = 25;
const SHP_MULTIPOINT_M: i32 = 28;

// -- ISO WKB Geometry Types --

const WKB_POINT: u32 = 1;
const WKB_LINESTRING: u32 = 2;
const WKB_POLYGON: u32 = 3;
const WKB_MULTIPOINT: u32 = 4;
const WKB_MULTILINESTRING: u32 = 5;
const WKB_MULTIPOLYGON: u32 = 6;

/// ISO WKB Z offset (add to base type for Z dimension).
const WKB_Z: u32 = 1000;
/// ISO WKB M offset (add to base type for M dimension).
const WKB_M: u32 = 2000;

/// SHP "no data" sentinel for M values - any M ≤ this is "no data".
const SHP_M_NO_DATA: f64 = -1e38;

/// WKB byte order: little-endian.
const WKB_LE: u8 = 1;

// -- Public API --

/// Convert SHP geometry bytes (Alteryx SpatialObj) to ISO WKB.
///
/// Returns `None` for null shapes (SHP type 0).
/// Returns `Some(wkb_bytes)` for valid geometries.
///
/// # Errors
///
/// Returns an error if the SHP data is malformed or uses an unsupported
/// shape type.
pub fn shp_to_wkb(shp: &[u8]) -> Result<Option<Vec<u8>>> {
    if shp.len() < 4 {
        return Err(spatial_err("SHP record too short (< 4 bytes)"));
    }

    let shape_type = read_i32_le(shp, 0);

    match shape_type {
        SHP_NULL => Ok(None),
        SHP_POINT => shp_point_to_wkb(shp),
        SHP_POLYLINE => shp_multiline_to_wkb(shp, false, false),
        SHP_POLYGON => shp_multipoly_to_wkb(shp, false, false),
        SHP_MULTIPOINT => shp_multipoint_to_wkb(shp, false, false),
        SHP_POINT_Z => shp_point_z_to_wkb(shp),
        SHP_POLYLINE_Z => shp_multiline_to_wkb(shp, true, false),
        SHP_POLYGON_Z => shp_multipoly_to_wkb(shp, true, false),
        SHP_MULTIPOINT_Z => shp_multipoint_to_wkb(shp, true, false),
        SHP_POINT_M => shp_point_m_to_wkb(shp),
        SHP_POLYLINE_M => shp_multiline_to_wkb(shp, false, true),
        SHP_POLYGON_M => shp_multipoly_to_wkb(shp, false, true),
        SHP_MULTIPOINT_M => shp_multipoint_to_wkb(shp, false, true),
        _ => Err(spatial_err(&format!(
            "unsupported SHP shape type: {shape_type}"
        ))),
    }
}

/// Convert ISO WKB geometry bytes to SHP format (Alteryx SpatialObj).
///
/// # Errors
///
/// Returns an error if the WKB data is malformed or uses an unsupported
/// geometry type.
pub fn wkb_to_shp(wkb: &[u8]) -> Result<Vec<u8>> {
    if wkb.len() < 5 {
        return Err(spatial_err("WKB record too short (< 5 bytes)"));
    }

    let byte_order = wkb[0];
    let is_le = byte_order == 1;
    let wkb_type = if is_le {
        read_u32_le(wkb, 1)
    } else {
        read_u32_be(wkb, 1)
    };

    // Determine dimensions from ISO WKB type
    let base_type = wkb_type % 1000;
    let has_z = matches!(wkb_type / 1000, 1 | 3);
    let has_m = matches!(wkb_type / 1000, 2 | 3);

    match base_type {
        1 => wkb_point_to_shp(wkb, is_le, has_z, has_m),
        2 => wkb_linestring_to_shp(wkb, is_le, has_z, has_m),
        3 => wkb_polygon_to_shp(wkb, is_le, has_z, has_m),
        4 => wkb_multipoint_to_shp(wkb, is_le, has_z, has_m),
        5 => wkb_multilinestring_to_shp(wkb, is_le, has_z, has_m),
        6 => wkb_multipolygon_to_shp(wkb, is_le, has_z, has_m),
        _ => Err(spatial_err(&format!(
            "unsupported WKB geometry type: {wkb_type}"
        ))),
    }
}

// -- SHP → WKB conversion helpers --

fn shp_point_to_wkb(shp: &[u8]) -> Result<Option<Vec<u8>>> {
    ensure_len(shp, 20, "Point")?;
    let x = read_f64_le(shp, 4);
    let y = read_f64_le(shp, 12);

    let mut out = Vec::with_capacity(21);
    out.push(WKB_LE);
    out.extend_from_slice(&WKB_POINT.to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    Ok(Some(out))
}

fn shp_point_z_to_wkb(shp: &[u8]) -> Result<Option<Vec<u8>>> {
    ensure_len(shp, 28, "PointZ")?; // at minimum: type + x + y + z = 4+8+8+8
    let x = read_f64_le(shp, 4);
    let y = read_f64_le(shp, 12);
    let z = read_f64_le(shp, 20);

    // M is optional in PointZ (depends on record length)
    let has_m = shp.len() >= 36;
    if has_m {
        let m = read_f64_le(shp, 28);
        if m > SHP_M_NO_DATA {
            // Include M → PointZM
            let mut out = Vec::with_capacity(37);
            out.push(WKB_LE);
            out.extend_from_slice(&(WKB_POINT + WKB_Z + WKB_M).to_le_bytes());
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
            out.extend_from_slice(&z.to_le_bytes());
            out.extend_from_slice(&m.to_le_bytes());
            return Ok(Some(out));
        }
    }

    // Z only
    let mut out = Vec::with_capacity(29);
    out.push(WKB_LE);
    out.extend_from_slice(&(WKB_POINT + WKB_Z).to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out.extend_from_slice(&z.to_le_bytes());
    Ok(Some(out))
}

fn shp_point_m_to_wkb(shp: &[u8]) -> Result<Option<Vec<u8>>> {
    ensure_len(shp, 28, "PointM")?;
    let x = read_f64_le(shp, 4);
    let y = read_f64_le(shp, 12);
    let m = read_f64_le(shp, 20);

    let mut out = Vec::with_capacity(29);
    out.push(WKB_LE);
    out.extend_from_slice(&(WKB_POINT + WKB_M).to_le_bytes());
    out.extend_from_slice(&x.to_le_bytes());
    out.extend_from_slice(&y.to_le_bytes());
    out.extend_from_slice(&m.to_le_bytes());
    Ok(Some(out))
}

/// Parse an SHP PolyLine/PolyLineZ/PolyLineM record and emit a WKB MultiLineString.
fn shp_multiline_to_wkb(shp: &[u8], has_z: bool, has_m: bool) -> Result<Option<Vec<u8>>> {
    let (num_parts, num_points, parts, points_offset) = parse_shp_parts_header(shp, has_z, has_m)?;

    // Read XY points
    let xy = read_xy_array(shp, points_offset, num_points)?;

    // Read Z and M arrays if present
    let z_array = if has_z {
        let z_offset = points_offset + num_points * 16;
        ensure_len(shp, z_offset + 16, "Z range")?; // z_range (2 doubles)
        let z_vals_offset = z_offset + 16;
        Some(read_f64_array(shp, z_vals_offset, num_points)?)
    } else {
        None
    };

    let m_array = if has_z {
        // For Z types, M is optional (at end, if record is long enough)
        let z_offset = points_offset + num_points * 16;
        let m_offset = z_offset + 16 + num_points * 8; // after z_range + z_array
        if shp.len() >= m_offset + 16 + num_points * 8 {
            let m_vals_offset = m_offset + 16;
            let ms = read_f64_array(shp, m_vals_offset, num_points)?;
            if ms.iter().any(|&v| v > SHP_M_NO_DATA) {
                Some(ms)
            } else {
                None
            }
        } else {
            None
        }
    } else if has_m {
        let m_offset = points_offset + num_points * 16;
        ensure_len(shp, m_offset + 16, "M range")?;
        let m_vals_offset = m_offset + 16;
        Some(read_f64_array(shp, m_vals_offset, num_points)?)
    } else {
        None
    };

    let wkb_type = multi_wkb_type(WKB_MULTILINESTRING, z_array.is_some(), m_array.is_some());
    let line_type = multi_wkb_type(WKB_LINESTRING, z_array.is_some(), m_array.is_some());

    let mut out = Vec::new();
    out.push(WKB_LE);
    out.extend_from_slice(&wkb_type.to_le_bytes());
    out.extend_from_slice(&(num_parts as u32).to_le_bytes());

    for part_idx in 0..num_parts {
        let start = parts[part_idx] as usize;
        let end = if part_idx + 1 < num_parts {
            parts[part_idx + 1] as usize
        } else {
            num_points
        };
        let count = end - start;

        out.push(WKB_LE);
        out.extend_from_slice(&line_type.to_le_bytes());
        out.extend_from_slice(&(count as u32).to_le_bytes());

        for i in start..end {
            out.extend_from_slice(&xy[i].0.to_le_bytes());
            out.extend_from_slice(&xy[i].1.to_le_bytes());
            if let Some(ref zs) = z_array {
                out.extend_from_slice(&zs[i].to_le_bytes());
            }
            if let Some(ref ms) = m_array {
                out.extend_from_slice(&ms[i].to_le_bytes());
            }
        }
    }

    Ok(Some(out))
}

/// Parse an SHP Polygon/PolygonZ/PolygonM record and emit a WKB MultiPolygon.
///
/// SHP polygons store all rings (exterior and interior) in a flat list.
/// We group them into polygons using the signed-area ring direction:
/// clockwise (positive area) = exterior ring, counterclockwise = hole.
fn shp_multipoly_to_wkb(shp: &[u8], has_z: bool, has_m: bool) -> Result<Option<Vec<u8>>> {
    let (num_parts, num_points, parts, points_offset) = parse_shp_parts_header(shp, has_z, has_m)?;

    let xy = read_xy_array(shp, points_offset, num_points)?;

    let z_array = if has_z {
        let z_offset = points_offset + num_points * 16;
        ensure_len(shp, z_offset + 16, "Z range")?;
        let z_vals_offset = z_offset + 16;
        Some(read_f64_array(shp, z_vals_offset, num_points)?)
    } else {
        None
    };

    let m_array = if has_z {
        let z_offset = points_offset + num_points * 16;
        let m_offset = z_offset + 16 + num_points * 8;
        if shp.len() >= m_offset + 16 + num_points * 8 {
            let m_vals_offset = m_offset + 16;
            let ms = read_f64_array(shp, m_vals_offset, num_points)?;
            if ms.iter().any(|&v| v > SHP_M_NO_DATA) {
                Some(ms)
            } else {
                None
            }
        } else {
            None
        }
    } else if has_m {
        let m_offset = points_offset + num_points * 16;
        ensure_len(shp, m_offset + 16, "M range")?;
        let m_vals_offset = m_offset + 16;
        Some(read_f64_array(shp, m_vals_offset, num_points)?)
    } else {
        None
    };

    // Extract ring boundaries
    let mut rings: Vec<(usize, usize)> = Vec::with_capacity(num_parts);
    for part_idx in 0..num_parts {
        let start = parts[part_idx] as usize;
        let end = if part_idx + 1 < num_parts {
            parts[part_idx + 1] as usize
        } else {
            num_points
        };
        rings.push((start, end));
    }

    // Group rings into polygons using signed area
    // Clockwise (positive signed area in SHP convention) = exterior ring
    // Counterclockwise (negative) = interior ring (hole)
    let mut polygons: Vec<Vec<(usize, usize)>> = Vec::new();
    for &(start, end) in &rings {
        let area = signed_area_2d(&xy[start..end]);
        if area >= 0.0 || polygons.is_empty() {
            // Exterior ring (clockwise) or first ring always starts a new polygon
            polygons.push(vec![(start, end)]);
        } else {
            // Interior ring (hole) - add to most recent polygon
            polygons.last_mut().unwrap().push((start, end));
        }
    }

    let wkb_multi_type = multi_wkb_type(WKB_MULTIPOLYGON, z_array.is_some(), m_array.is_some());
    let wkb_poly_type = multi_wkb_type(WKB_POLYGON, z_array.is_some(), m_array.is_some());

    let mut out = Vec::new();
    out.push(WKB_LE);
    out.extend_from_slice(&wkb_multi_type.to_le_bytes());
    out.extend_from_slice(&(polygons.len() as u32).to_le_bytes());

    for poly_rings in &polygons {
        out.push(WKB_LE);
        out.extend_from_slice(&wkb_poly_type.to_le_bytes());
        out.extend_from_slice(&(poly_rings.len() as u32).to_le_bytes());

        for &(start, end) in poly_rings {
            let count = end - start;
            out.extend_from_slice(&(count as u32).to_le_bytes());
            for i in start..end {
                out.extend_from_slice(&xy[i].0.to_le_bytes());
                out.extend_from_slice(&xy[i].1.to_le_bytes());
                if let Some(ref zs) = z_array {
                    out.extend_from_slice(&zs[i].to_le_bytes());
                }
                if let Some(ref ms) = m_array {
                    out.extend_from_slice(&ms[i].to_le_bytes());
                }
            }
        }
    }

    Ok(Some(out))
}

/// Parse an SHP MultiPoint/MultiPointZ/MultiPointM record and emit a WKB MultiPoint.
fn shp_multipoint_to_wkb(shp: &[u8], has_z: bool, has_m: bool) -> Result<Option<Vec<u8>>> {
    ensure_len(shp, 40, "MultiPoint header")?;
    // Skip bbox (32 bytes at offset 4)
    let num_points = read_i32_le(shp, 36) as usize;
    let points_offset = 40;

    let xy = read_xy_array(shp, points_offset, num_points)?;

    let z_array = if has_z {
        let z_offset = points_offset + num_points * 16;
        ensure_len(shp, z_offset + 16, "Z range")?;
        let z_vals_offset = z_offset + 16;
        Some(read_f64_array(shp, z_vals_offset, num_points)?)
    } else {
        None
    };

    let m_array = if has_z {
        let z_offset = points_offset + num_points * 16;
        let m_offset = z_offset + 16 + num_points * 8;
        if shp.len() >= m_offset + 16 + num_points * 8 {
            let m_vals_offset = m_offset + 16;
            let ms = read_f64_array(shp, m_vals_offset, num_points)?;
            if ms.iter().any(|&v| v > SHP_M_NO_DATA) {
                Some(ms)
            } else {
                None
            }
        } else {
            None
        }
    } else if has_m {
        let m_offset = points_offset + num_points * 16;
        ensure_len(shp, m_offset + 16, "M range")?;
        let m_vals_offset = m_offset + 16;
        Some(read_f64_array(shp, m_vals_offset, num_points)?)
    } else {
        None
    };

    let wkb_multi_type = multi_wkb_type(WKB_MULTIPOINT, z_array.is_some(), m_array.is_some());
    let wkb_pt_type = multi_wkb_type(WKB_POINT, z_array.is_some(), m_array.is_some());

    let mut out = Vec::new();
    out.push(WKB_LE);
    out.extend_from_slice(&wkb_multi_type.to_le_bytes());
    out.extend_from_slice(&(num_points as u32).to_le_bytes());

    for i in 0..num_points {
        out.push(WKB_LE);
        out.extend_from_slice(&wkb_pt_type.to_le_bytes());
        out.extend_from_slice(&xy[i].0.to_le_bytes());
        out.extend_from_slice(&xy[i].1.to_le_bytes());
        if let Some(ref zs) = z_array {
            out.extend_from_slice(&zs[i].to_le_bytes());
        }
        if let Some(ref ms) = m_array {
            out.extend_from_slice(&ms[i].to_le_bytes());
        }
    }

    Ok(Some(out))
}

// -- WKB → SHP conversion helpers --

fn wkb_point_to_shp(wkb: &[u8], is_le: bool, has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    let coord_start = 5; // after byte_order + type
    let dim = 2 + has_z as usize + has_m as usize;
    ensure_len(wkb, coord_start + dim * 8, "WKB Point")?;

    let x = read_f64(wkb, coord_start, is_le);
    let y = read_f64(wkb, coord_start + 8, is_le);

    if has_z && has_m {
        let z = read_f64(wkb, coord_start + 16, is_le);
        let m = read_f64(wkb, coord_start + 24, is_le);
        let mut out = Vec::with_capacity(36);
        out.extend_from_slice(&SHP_POINT_Z.to_le_bytes());
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
        out.extend_from_slice(&z.to_le_bytes());
        out.extend_from_slice(&m.to_le_bytes());
        Ok(out)
    } else if has_z {
        let z = read_f64(wkb, coord_start + 16, is_le);
        let mut out = Vec::with_capacity(36);
        out.extend_from_slice(&SHP_POINT_Z.to_le_bytes());
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
        out.extend_from_slice(&z.to_le_bytes());
        // M = no data
        out.extend_from_slice(&(-1.0e40_f64).to_le_bytes());
        Ok(out)
    } else if has_m {
        let m = read_f64(wkb, coord_start + 16, is_le);
        let mut out = Vec::with_capacity(28);
        out.extend_from_slice(&SHP_POINT_M.to_le_bytes());
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
        out.extend_from_slice(&m.to_le_bytes());
        Ok(out)
    } else {
        let mut out = Vec::with_capacity(20);
        out.extend_from_slice(&SHP_POINT.to_le_bytes());
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&y.to_le_bytes());
        Ok(out)
    }
}

/// Convert a WKB LineString to SHP PolyLine (single part).
fn wkb_linestring_to_shp(wkb: &[u8], is_le: bool, has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    let (coords, _dim) = parse_wkb_linestring_coords(wkb, 5, is_le, has_z, has_m)?;
    build_shp_polyline(&[&coords], has_z, has_m)
}

/// Convert a WKB Polygon to SHP Polygon (rings become parts).
fn wkb_polygon_to_shp(wkb: &[u8], is_le: bool, has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    let rings = parse_wkb_polygon_rings(wkb, 5, is_le, has_z, has_m)?;
    let ring_refs: Vec<&[CoordND]> = rings.iter().map(|r| r.as_slice()).collect();
    build_shp_polygon(&ring_refs, has_z, has_m)
}

/// Convert a WKB MultiPoint to SHP MultiPoint.
fn wkb_multipoint_to_shp(wkb: &[u8], is_le: bool, has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    ensure_len(wkb, 9, "WKB MultiPoint")?;
    let num_points = read_u32(wkb, 5, is_le) as usize;
    let mut offset = 9;
    let mut coords: Vec<CoordND> = Vec::with_capacity(num_points);

    for _ in 0..num_points {
        ensure_len(wkb, offset + 5, "WKB MultiPoint sub-point header")?;
        let sub_le = wkb[offset] == 1;
        let sub_type = read_u32(wkb, offset + 1, sub_le);
        let sub_base = sub_type % 1000;
        let sub_has_z = matches!(sub_type / 1000, 1 | 3);
        let sub_has_m = matches!(sub_type / 1000, 2 | 3);
        if sub_base != 1 {
            return Err(spatial_err("WKB MultiPoint contains non-Point geometry"));
        }
        let dim = 2 + sub_has_z as usize + sub_has_m as usize;
        ensure_len(wkb, offset + 5 + dim * 8, "WKB MultiPoint sub-point coords")?;
        let x = read_f64(wkb, offset + 5, sub_le);
        let y = read_f64(wkb, offset + 13, sub_le);
        let z = if sub_has_z {
            read_f64(wkb, offset + 21, sub_le)
        } else {
            0.0
        };
        let m = if sub_has_m {
            let m_off = if sub_has_z { offset + 29 } else { offset + 21 };
            read_f64(wkb, m_off, sub_le)
        } else {
            -1.0e40
        };
        coords.push(CoordND { x, y, z, m });
        offset += 5 + dim * 8;
    }

    build_shp_multipoint(&coords, has_z, has_m)
}

/// Convert a WKB MultiLineString to SHP PolyLine (parts = linestrings).
fn wkb_multilinestring_to_shp(
    wkb: &[u8],
    is_le: bool,
    has_z: bool,
    has_m: bool,
) -> Result<Vec<u8>> {
    ensure_len(wkb, 9, "WKB MultiLineString")?;
    let num_lines = read_u32(wkb, 5, is_le) as usize;
    let mut offset = 9;
    let mut parts: Vec<Vec<CoordND>> = Vec::with_capacity(num_lines);

    for _ in 0..num_lines {
        ensure_len(wkb, offset + 5, "WKB MultiLineString sub-linestring header")?;
        let sub_le = wkb[offset] == 1;
        // skip type validation for flexibility
        offset += 5; // byte_order + type
        let (coords, _) = parse_wkb_ring_coords(wkb, offset, sub_le, has_z, has_m)?;
        offset += 4 + coords.len() * (2 + has_z as usize + has_m as usize) * 8;
        parts.push(coords);
    }

    build_shp_polyline(
        &parts.iter().map(|p| p.as_slice()).collect::<Vec<_>>(),
        has_z,
        has_m,
    )
}

/// Convert a WKB MultiPolygon to SHP Polygon (all rings merged).
fn wkb_multipolygon_to_shp(wkb: &[u8], is_le: bool, has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    ensure_len(wkb, 9, "WKB MultiPolygon")?;
    let num_polygons = read_u32(wkb, 5, is_le) as usize;

    let mut all_rings: Vec<Vec<CoordND>> = Vec::new();
    let mut off = 9;
    let dim = 2 + has_z as usize + has_m as usize;

    for _ in 0..num_polygons {
        ensure_len(wkb, off + 5, "WKB MultiPolygon sub-polygon header")?;
        let sub_le = wkb[off] == 1;
        off += 5; // byte_order + type
        ensure_len(wkb, off + 4, "WKB Polygon ring count")?;
        let num_rings = read_u32(wkb, off, sub_le) as usize;
        off += 4;
        for _ in 0..num_rings {
            ensure_len(wkb, off + 4, "WKB ring point count")?;
            let num_pts = read_u32(wkb, off, sub_le) as usize;
            off += 4;
            let coord_bytes = num_pts * dim * 8;
            ensure_len(wkb, off + coord_bytes, "WKB ring coords")?;
            let mut coords = Vec::with_capacity(num_pts);
            for j in 0..num_pts {
                let base = off + j * dim * 8;
                let x = read_f64(wkb, base, sub_le);
                let y = read_f64(wkb, base + 8, sub_le);
                let z = if has_z {
                    read_f64(wkb, base + 16, sub_le)
                } else {
                    0.0
                };
                let m = if has_m {
                    let m_off = if has_z { base + 24 } else { base + 16 };
                    read_f64(wkb, m_off, sub_le)
                } else {
                    -1.0e40
                };
                coords.push(CoordND { x, y, z, m });
            }
            off += coord_bytes;
            all_rings.push(coords);
        }
    }

    let ring_refs: Vec<&[CoordND]> = all_rings.iter().map(|r| r.as_slice()).collect();
    build_shp_polygon(&ring_refs, has_z, has_m)
}

// -- SHP output builders --

/// N-dimensional coordinate (up to XYZM).
#[derive(Clone, Copy)]
struct CoordND {
    x: f64,
    y: f64,
    z: f64,
    m: f64,
}

/// Build an SHP PolyLine from a list of parts (each part = list of coords).
fn build_shp_polyline(parts: &[&[CoordND]], has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    let shp_type = if has_z {
        SHP_POLYLINE_Z
    } else if has_m {
        SHP_POLYLINE_M
    } else {
        SHP_POLYLINE
    };

    let num_parts = parts.len();
    let num_points: usize = parts.iter().map(|p| p.len()).sum();
    let all_coords: Vec<&CoordND> = parts.iter().flat_map(|p| p.iter()).collect();

    let (xmin, ymin, xmax, ymax) = bbox_2d(&all_coords);

    // Header: type(4) + bbox(32) + num_parts(4) + num_points(4) + parts(num_parts*4)
    // + points(num_points*16)
    let base_size = 4 + 32 + 4 + 4 + num_parts * 4 + num_points * 16;
    let z_size = if has_z { 16 + num_points * 8 } else { 0 }; // z_range + z_array
    let m_size = if has_z || has_m {
        16 + num_points * 8
    } else {
        0
    }; // m_range + m_array

    let mut out = Vec::with_capacity(base_size + z_size + m_size);

    // Shape type
    out.extend_from_slice(&shp_type.to_le_bytes());
    // Bounding box
    out.extend_from_slice(&xmin.to_le_bytes());
    out.extend_from_slice(&ymin.to_le_bytes());
    out.extend_from_slice(&xmax.to_le_bytes());
    out.extend_from_slice(&ymax.to_le_bytes());
    // Num parts, num points
    out.extend_from_slice(&(num_parts as i32).to_le_bytes());
    out.extend_from_slice(&(num_points as i32).to_le_bytes());
    // Part indices
    let mut idx = 0i32;
    for part in parts {
        out.extend_from_slice(&idx.to_le_bytes());
        idx += part.len() as i32;
    }
    // XY points
    for c in &all_coords {
        out.extend_from_slice(&c.x.to_le_bytes());
        out.extend_from_slice(&c.y.to_le_bytes());
    }

    // Z range + Z values
    if has_z {
        let (zmin, zmax) = range_1d(&all_coords, |c| c.z);
        out.extend_from_slice(&zmin.to_le_bytes());
        out.extend_from_slice(&zmax.to_le_bytes());
        for c in &all_coords {
            out.extend_from_slice(&c.z.to_le_bytes());
        }
    }

    // M range + M values
    if has_z || has_m {
        let (mmin, mmax) = range_1d(&all_coords, |c| c.m);
        out.extend_from_slice(&mmin.to_le_bytes());
        out.extend_from_slice(&mmax.to_le_bytes());
        for c in &all_coords {
            out.extend_from_slice(&c.m.to_le_bytes());
        }
    }

    Ok(out)
}

/// Build an SHP Polygon from a list of rings (each ring = list of coords).
fn build_shp_polygon(rings: &[&[CoordND]], has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    let shp_type = if has_z {
        SHP_POLYGON_Z
    } else if has_m {
        SHP_POLYGON_M
    } else {
        SHP_POLYGON
    };

    let num_parts = rings.len();
    let num_points: usize = rings.iter().map(|r| r.len()).sum();
    let all_coords: Vec<&CoordND> = rings.iter().flat_map(|r| r.iter()).collect();

    let (xmin, ymin, xmax, ymax) = bbox_2d(&all_coords);

    let base_size = 4 + 32 + 4 + 4 + num_parts * 4 + num_points * 16;
    let z_size = if has_z { 16 + num_points * 8 } else { 0 };
    let m_size = if has_z || has_m {
        16 + num_points * 8
    } else {
        0
    };

    let mut out = Vec::with_capacity(base_size + z_size + m_size);

    out.extend_from_slice(&shp_type.to_le_bytes());
    out.extend_from_slice(&xmin.to_le_bytes());
    out.extend_from_slice(&ymin.to_le_bytes());
    out.extend_from_slice(&xmax.to_le_bytes());
    out.extend_from_slice(&ymax.to_le_bytes());
    out.extend_from_slice(&(num_parts as i32).to_le_bytes());
    out.extend_from_slice(&(num_points as i32).to_le_bytes());

    let mut idx = 0i32;
    for ring in rings {
        out.extend_from_slice(&idx.to_le_bytes());
        idx += ring.len() as i32;
    }

    for c in &all_coords {
        out.extend_from_slice(&c.x.to_le_bytes());
        out.extend_from_slice(&c.y.to_le_bytes());
    }

    if has_z {
        let (zmin, zmax) = range_1d(&all_coords, |c| c.z);
        out.extend_from_slice(&zmin.to_le_bytes());
        out.extend_from_slice(&zmax.to_le_bytes());
        for c in &all_coords {
            out.extend_from_slice(&c.z.to_le_bytes());
        }
    }

    if has_z || has_m {
        let (mmin, mmax) = range_1d(&all_coords, |c| c.m);
        out.extend_from_slice(&mmin.to_le_bytes());
        out.extend_from_slice(&mmax.to_le_bytes());
        for c in &all_coords {
            out.extend_from_slice(&c.m.to_le_bytes());
        }
    }

    Ok(out)
}

/// Build an SHP MultiPoint from a list of coordinates.
fn build_shp_multipoint(coords: &[CoordND], has_z: bool, has_m: bool) -> Result<Vec<u8>> {
    let shp_type = if has_z {
        SHP_MULTIPOINT_Z
    } else if has_m {
        SHP_MULTIPOINT_M
    } else {
        SHP_MULTIPOINT
    };

    let num_points = coords.len();
    let coord_refs: Vec<&CoordND> = coords.iter().collect();
    let (xmin, ymin, xmax, ymax) = bbox_2d(&coord_refs);

    let base_size = 4 + 32 + 4 + num_points * 16;
    let z_size = if has_z { 16 + num_points * 8 } else { 0 };
    let m_size = if has_z || has_m {
        16 + num_points * 8
    } else {
        0
    };

    let mut out = Vec::with_capacity(base_size + z_size + m_size);

    out.extend_from_slice(&shp_type.to_le_bytes());
    out.extend_from_slice(&xmin.to_le_bytes());
    out.extend_from_slice(&ymin.to_le_bytes());
    out.extend_from_slice(&xmax.to_le_bytes());
    out.extend_from_slice(&ymax.to_le_bytes());
    out.extend_from_slice(&(num_points as i32).to_le_bytes());

    for c in coords {
        out.extend_from_slice(&c.x.to_le_bytes());
        out.extend_from_slice(&c.y.to_le_bytes());
    }

    if has_z {
        let (zmin, zmax) = range_1d(&coord_refs, |c| c.z);
        out.extend_from_slice(&zmin.to_le_bytes());
        out.extend_from_slice(&zmax.to_le_bytes());
        for c in coords {
            out.extend_from_slice(&c.z.to_le_bytes());
        }
    }

    if has_z || has_m {
        let (mmin, mmax) = range_1d(&coord_refs, |c| c.m);
        out.extend_from_slice(&mmin.to_le_bytes());
        out.extend_from_slice(&mmax.to_le_bytes());
        for c in coords {
            out.extend_from_slice(&c.m.to_le_bytes());
        }
    }

    Ok(out)
}

// -- SHP parsing helpers --

/// Parse the common header for SHP PolyLine and Polygon records.
///
/// Returns (num_parts, num_points, parts_array, points_data_offset).
fn parse_shp_parts_header(
    shp: &[u8],
    _has_z: bool,
    _has_m: bool,
) -> Result<(usize, usize, Vec<i32>, usize)> {
    ensure_len(shp, 44, "PolyLine/Polygon header")?;
    // offset 4: bbox (32 bytes, skip)
    let num_parts = read_i32_le(shp, 36) as usize;
    let num_points = read_i32_le(shp, 40) as usize;

    let parts_offset = 44;
    ensure_len(shp, parts_offset + num_parts * 4, "parts array")?;
    let mut parts = Vec::with_capacity(num_parts);
    for i in 0..num_parts {
        parts.push(read_i32_le(shp, parts_offset + i * 4));
    }

    let points_offset = parts_offset + num_parts * 4;
    Ok((num_parts, num_points, parts, points_offset))
}

/// Read an array of XY point pairs from SHP data.
fn read_xy_array(shp: &[u8], offset: usize, count: usize) -> Result<Vec<(f64, f64)>> {
    let needed = offset + count * 16;
    ensure_len(shp, needed, "XY points")?;
    let mut pts = Vec::with_capacity(count);
    for i in 0..count {
        let base = offset + i * 16;
        pts.push((read_f64_le(shp, base), read_f64_le(shp, base + 8)));
    }
    Ok(pts)
}

/// Read an array of f64 values from SHP data.
fn read_f64_array(shp: &[u8], offset: usize, count: usize) -> Result<Vec<f64>> {
    let needed = offset + count * 8;
    ensure_len(shp, needed, "f64 array")?;
    let mut vals = Vec::with_capacity(count);
    for i in 0..count {
        vals.push(read_f64_le(shp, offset + i * 8));
    }
    Ok(vals)
}

// -- WKB parsing helpers --

/// Parse a WKB LineString's coordinate list (starting at `offset` which points
/// to the num_points field). Returns the coordinates and the number of
/// dimensions.
fn parse_wkb_linestring_coords(
    wkb: &[u8],
    offset: usize,
    is_le: bool,
    has_z: bool,
    has_m: bool,
) -> Result<(Vec<CoordND>, usize)> {
    parse_wkb_ring_coords(wkb, offset, is_le, has_z, has_m)
}

/// Parse a single WKB ring (shared between LineString and Polygon ring parsing).
/// `offset` points to the num_points u32.
fn parse_wkb_ring_coords(
    wkb: &[u8],
    offset: usize,
    is_le: bool,
    has_z: bool,
    has_m: bool,
) -> Result<(Vec<CoordND>, usize)> {
    ensure_len(wkb, offset + 4, "WKB ring point count")?;
    let num_pts = read_u32(wkb, offset, is_le) as usize;
    let dim = 2 + has_z as usize + has_m as usize;
    let coords_offset = offset + 4;
    let coord_bytes = num_pts * dim * 8;
    ensure_len(wkb, coords_offset + coord_bytes, "WKB ring coords")?;

    let mut coords = Vec::with_capacity(num_pts);
    for i in 0..num_pts {
        let base = coords_offset + i * dim * 8;
        let x = read_f64(wkb, base, is_le);
        let y = read_f64(wkb, base + 8, is_le);
        let z = if has_z {
            read_f64(wkb, base + 16, is_le)
        } else {
            0.0
        };
        let m = if has_m {
            let m_off = if has_z { base + 24 } else { base + 16 };
            read_f64(wkb, m_off, is_le)
        } else {
            -1.0e40
        };
        coords.push(CoordND { x, y, z, m });
    }

    Ok((coords, dim))
}

/// Parse a WKB Polygon's rings. `offset` points to the num_rings u32.
fn parse_wkb_polygon_rings(
    wkb: &[u8],
    offset: usize,
    is_le: bool,
    has_z: bool,
    has_m: bool,
) -> Result<Vec<Vec<CoordND>>> {
    ensure_len(wkb, offset + 4, "WKB Polygon ring count")?;
    let num_rings = read_u32(wkb, offset, is_le) as usize;
    let dim = 2 + has_z as usize + has_m as usize;
    let mut off = offset + 4;
    let mut rings = Vec::with_capacity(num_rings);

    for _ in 0..num_rings {
        let (coords, _) = parse_wkb_ring_coords(wkb, off, is_le, has_z, has_m)?;
        off += 4 + coords.len() * dim * 8;
        rings.push(coords);
    }

    Ok(rings)
}

// -- Geometry math helpers --

/// Compute the signed area of a 2D ring (Shoelace formula).
///
/// Positive = clockwise (exterior in SHP convention).
/// Negative = counterclockwise (hole in SHP convention).
fn signed_area_2d(ring: &[(f64, f64)]) -> f64 {
    let n = ring.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += ring[i].0 * ring[j].1;
        area -= ring[j].0 * ring[i].1;
    }
    area / 2.0
}

/// Compute the 2D bounding box of a set of coordinates.
fn bbox_2d(coords: &[&CoordND]) -> (f64, f64, f64, f64) {
    if coords.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let mut xmin = f64::INFINITY;
    let mut ymin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for c in coords {
        xmin = xmin.min(c.x);
        ymin = ymin.min(c.y);
        xmax = xmax.max(c.x);
        ymax = ymax.max(c.y);
    }
    (xmin, ymin, xmax, ymax)
}

/// Compute the range (min, max) of a single dimension.
fn range_1d<F: Fn(&CoordND) -> f64>(coords: &[&CoordND], f: F) -> (f64, f64) {
    if coords.is_empty() {
        return (0.0, 0.0);
    }
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for c in coords {
        let v = f(c);
        min = min.min(v);
        max = max.max(v);
    }
    (min, max)
}

// -- Low-level binary helpers --

#[inline]
fn read_i32_le(buf: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap())
}

#[inline]
fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap())
}

#[inline]
fn read_u32_be(buf: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap())
}

#[inline]
fn read_u32(buf: &[u8], offset: usize, is_le: bool) -> u32 {
    if is_le {
        read_u32_le(buf, offset)
    } else {
        read_u32_be(buf, offset)
    }
}

#[inline]
fn read_f64_le(buf: &[u8], offset: usize) -> f64 {
    f64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap())
}

#[inline]
fn read_f64_be(buf: &[u8], offset: usize) -> f64 {
    f64::from_be_bytes(buf[offset..offset + 8].try_into().unwrap())
}

#[inline]
fn read_f64(buf: &[u8], offset: usize, is_le: bool) -> f64 {
    if is_le {
        read_f64_le(buf, offset)
    } else {
        read_f64_be(buf, offset)
    }
}

#[inline]
fn ensure_len(buf: &[u8], needed: usize, context: &str) -> Result<()> {
    if buf.len() < needed {
        Err(spatial_err(&format!(
            "{context}: need {needed} bytes, have {}",
            buf.len()
        )))
    } else {
        Ok(())
    }
}

fn spatial_err(msg: &str) -> YxdbError {
    YxdbError::ConversionError(format!("spatial: {msg}"))
}

/// Compute the WKB geometry type with Z/M flags.
#[inline]
fn multi_wkb_type(base: u32, has_z: bool, has_m: bool) -> u32 {
    let mut t = base;
    if has_z && has_m {
        t += WKB_Z + WKB_M;
    } else if has_z {
        t += WKB_Z;
    } else if has_m {
        t += WKB_M;
    }
    t
}

// -- Spatial Mode --

/// Controls how `SpatialObj` columns are represented after reading (or
/// expected before writing).
///
/// | Mode       | Read behaviour                          | Write behaviour                         |
/// |------------|-----------------------------------------|-----------------------------------------|
/// | `Raw`      | Keep the internal SHP bytes as `Binary`  | N/A - spatial columns are raw SHP       |
/// | `Wkb`      | Decode SHP → ISO WKB `Binary`            | Encode WKB → SHP for `SpatialObj` fields |
/// | `GeoArrow` | Decode SHP → ISO WKB, tag as GeoArrow   | Same as Wkb (WKB → SHP conversion)     |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpatialMode {
    /// Keep SHP bytes as-is (Alteryx internal format).
    Raw,
    /// Convert to/from ISO Well-Known Binary (compatible with Shapely,
    /// GeoPandas, PostGIS, GDAL, etc.).
    #[default]
    Wkb,
    /// Convert to ISO WKB and tag columns with GeoArrow extension type
    /// metadata (`geoarrow.wkb`). This enables seamless interop with
    /// GeoArrow-aware libraries (lonboard, leafmap, DuckDB Spatial, etc.).
    ///
    /// On read, the resulting columns are WKB Binary with GeoArrow metadata.
    /// On write, behaves the same as [`Wkb`](Self::Wkb).
    GeoArrow,
}

// -- DataFrame-level conversion --

use crate::field::{FieldMeta, FieldType};
use polars::prelude::*;

/// Return the names of all `SpatialObj` columns in the field metadata.
pub fn spatial_column_names(fields: &[FieldMeta]) -> Vec<String> {
    fields
        .iter()
        .filter(|f| f.field_type == FieldType::SpatialObj)
        .map(|f| f.name.clone())
        .collect()
}

/// Convert all `SpatialObj` columns in a DataFrame from SHP to WKB binary.
///
/// Non-spatial columns are passed through unchanged.
pub fn convert_spatial_columns_to_wkb(df: DataFrame, fields: &[FieldMeta]) -> Result<DataFrame> {
    let mut columns: Vec<Column> = Vec::with_capacity(df.width());
    let height = df.height();

    for col in df.columns() {
        let is_spatial = fields.iter().any(|f| {
            f.name.as_str() == col.name().as_str() && f.field_type == FieldType::SpatialObj
        });

        if is_spatial {
            let binary = col.binary().map_err(|e| {
                YxdbError::ConversionError(format!("expected Binary column for SpatialObj: {e}"))
            })?;
            let converted: BinaryChunked = binary
                .into_iter()
                .map(|opt_bytes| match opt_bytes {
                    None => Ok(None),
                    Some([]) => Ok(None),
                    Some(shp_bytes) => match shp_to_wkb(shp_bytes)? {
                        None => Ok(None),
                        Some(wkb) => Ok(Some(wkb)),
                    },
                })
                .collect::<Result<Vec<Option<Vec<u8>>>>>()?
                .into_iter()
                .map(|opt: Option<Vec<u8>>| opt.map(|v| v.into_iter().collect::<Vec<u8>>()))
                .collect::<BinaryChunked>();
            let series = converted.with_name(col.name().clone()).into_series();
            columns.push(series.into());
        } else {
            columns.push(col.clone());
        }
    }

    DataFrame::new(height, columns).map_err(|e| YxdbError::ConversionError(e.to_string()))
}

/// Convert all `SpatialObj` columns in a DataFrame from WKB back to SHP binary.
///
/// This is used before writing: any Binary column targeted at a SpatialObj field
/// is assumed to be WKB and is converted to SHP format.
pub fn convert_spatial_columns_to_shp(df: &DataFrame, fields: &[FieldMeta]) -> Result<DataFrame> {
    let mut columns: Vec<Column> = Vec::with_capacity(df.width());
    let height = df.height();

    for col in df.columns() {
        let is_spatial = fields.iter().any(|f| {
            f.name.as_str() == col.name().as_str() && f.field_type == FieldType::SpatialObj
        });

        if is_spatial {
            let binary = col.binary().map_err(|e| {
                YxdbError::ConversionError(format!("expected Binary column for SpatialObj: {e}"))
            })?;
            let converted: Vec<Option<Vec<u8>>> = binary
                .into_iter()
                .map(|opt_bytes| match opt_bytes {
                    None => Ok(None),
                    Some([]) => Ok(None),
                    Some(wkb_bytes) => Ok(Some(wkb_to_shp(wkb_bytes)?)),
                })
                .collect::<Result<Vec<Option<Vec<u8>>>>>()?;
            let values: Vec<Option<&[u8]>> = converted
                .iter()
                .map(|opt: &Option<Vec<u8>>| opt.as_deref())
                .collect();
            let series = Series::new(col.name().clone(), values);
            columns.push(series.into());
        } else {
            columns.push(col.clone());
        }
    }

    DataFrame::new(height, columns).map_err(|e| YxdbError::ConversionError(e.to_string()))
}

// -- Tests --

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: create a DataFrame from columns, inferring height.
    fn test_df(columns: Vec<Column>) -> DataFrame {
        let h = columns.first().map_or(0, |c| c.len());
        DataFrame::new(h, columns).unwrap()
    }

    #[test]
    fn null_shape() {
        let shp = 0i32.to_le_bytes();
        assert_eq!(shp_to_wkb(&shp).unwrap(), None);
    }

    #[test]
    fn point_roundtrip() {
        // Build SHP Point
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_POINT.to_le_bytes());
        shp.extend_from_slice(&1.5f64.to_le_bytes());
        shp.extend_from_slice(&2.5f64.to_le_bytes());

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();

        // Verify WKB structure
        assert_eq!(wkb[0], WKB_LE);
        assert_eq!(read_u32_le(&wkb, 1), WKB_POINT);
        assert_eq!(read_f64_le(&wkb, 5), 1.5);
        assert_eq!(read_f64_le(&wkb, 13), 2.5);

        // Round-trip back to SHP
        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(shp, shp2);
    }

    #[test]
    fn point_z_roundtrip() {
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_POINT_Z.to_le_bytes());
        shp.extend_from_slice(&10.0f64.to_le_bytes());
        shp.extend_from_slice(&20.0f64.to_le_bytes());
        shp.extend_from_slice(&30.0f64.to_le_bytes());
        shp.extend_from_slice(&(-1.0e40f64).to_le_bytes()); // no-data M

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();
        // Should be PointZ (no M since sentinel)
        assert_eq!(read_u32_le(&wkb, 1), WKB_POINT + WKB_Z);
        assert_eq!(read_f64_le(&wkb, 5), 10.0);
        assert_eq!(read_f64_le(&wkb, 13), 20.0);
        assert_eq!(read_f64_le(&wkb, 21), 30.0);

        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp2, 0), SHP_POINT_Z);
        assert_eq!(read_f64_le(&shp2, 4), 10.0);
        assert_eq!(read_f64_le(&shp2, 12), 20.0);
        assert_eq!(read_f64_le(&shp2, 20), 30.0);
    }

    #[test]
    fn point_zm_roundtrip() {
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_POINT_Z.to_le_bytes());
        shp.extend_from_slice(&1.0f64.to_le_bytes());
        shp.extend_from_slice(&2.0f64.to_le_bytes());
        shp.extend_from_slice(&3.0f64.to_le_bytes());
        shp.extend_from_slice(&4.0f64.to_le_bytes()); // meaningful M

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();
        assert_eq!(read_u32_le(&wkb, 1), WKB_POINT + WKB_Z + WKB_M);
        assert_eq!(read_f64_le(&wkb, 5), 1.0);
        assert_eq!(read_f64_le(&wkb, 13), 2.0);
        assert_eq!(read_f64_le(&wkb, 21), 3.0);
        assert_eq!(read_f64_le(&wkb, 29), 4.0);

        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp2, 0), SHP_POINT_Z);
        assert_eq!(read_f64_le(&shp2, 4), 1.0);
        assert_eq!(read_f64_le(&shp2, 12), 2.0);
        assert_eq!(read_f64_le(&shp2, 20), 3.0);
        assert_eq!(read_f64_le(&shp2, 28), 4.0);
    }

    #[test]
    fn point_m_roundtrip() {
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_POINT_M.to_le_bytes());
        shp.extend_from_slice(&5.0f64.to_le_bytes());
        shp.extend_from_slice(&6.0f64.to_le_bytes());
        shp.extend_from_slice(&7.0f64.to_le_bytes());

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();
        assert_eq!(read_u32_le(&wkb, 1), WKB_POINT + WKB_M);

        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(shp, shp2);
    }

    #[test]
    fn polyline_single_part_roundtrip() {
        // SHP Polyline with 1 part, 3 points
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_POLYLINE.to_le_bytes());
        // bbox: (0,0)-(2,2)
        shp.extend_from_slice(&0.0f64.to_le_bytes()); // xmin
        shp.extend_from_slice(&0.0f64.to_le_bytes()); // ymin
        shp.extend_from_slice(&2.0f64.to_le_bytes()); // xmax
        shp.extend_from_slice(&2.0f64.to_le_bytes()); // ymax
        shp.extend_from_slice(&1i32.to_le_bytes()); // num_parts
        shp.extend_from_slice(&3i32.to_le_bytes()); // num_points
        shp.extend_from_slice(&0i32.to_le_bytes()); // part[0]
                                                    // Points
        for &(x, y) in &[(0.0f64, 0.0f64), (1.0, 1.0), (2.0, 2.0)] {
            shp.extend_from_slice(&x.to_le_bytes());
            shp.extend_from_slice(&y.to_le_bytes());
        }

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();
        assert_eq!(read_u32_le(&wkb, 1), WKB_MULTILINESTRING);

        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp2, 0), SHP_POLYLINE);
        // Same number of points
        assert_eq!(read_i32_le(&shp2, 40), 3);
    }

    #[test]
    fn polygon_single_ring_roundtrip() {
        // SHP Polygon with 1 ring (triangle, clockwise)
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_POLYGON.to_le_bytes());
        shp.extend_from_slice(&0.0f64.to_le_bytes()); // xmin
        shp.extend_from_slice(&0.0f64.to_le_bytes()); // ymin
        shp.extend_from_slice(&4.0f64.to_le_bytes()); // xmax
        shp.extend_from_slice(&3.0f64.to_le_bytes()); // ymax
        shp.extend_from_slice(&1i32.to_le_bytes()); // num_parts
        shp.extend_from_slice(&4i32.to_le_bytes()); // num_points (closed ring)
        shp.extend_from_slice(&0i32.to_le_bytes()); // part[0]
                                                    // Clockwise triangle
        for &(x, y) in &[(0.0f64, 0.0f64), (4.0, 0.0), (2.0, 3.0), (0.0, 0.0)] {
            shp.extend_from_slice(&x.to_le_bytes());
            shp.extend_from_slice(&y.to_le_bytes());
        }

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();
        assert_eq!(read_u32_le(&wkb, 1), WKB_MULTIPOLYGON);

        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp2, 0), SHP_POLYGON);
        assert_eq!(read_i32_le(&shp2, 40), 4);
    }

    #[test]
    fn multipoint_roundtrip() {
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_MULTIPOINT.to_le_bytes());
        shp.extend_from_slice(&1.0f64.to_le_bytes()); // xmin
        shp.extend_from_slice(&2.0f64.to_le_bytes()); // ymin
        shp.extend_from_slice(&3.0f64.to_le_bytes()); // xmax
        shp.extend_from_slice(&4.0f64.to_le_bytes()); // ymax
        shp.extend_from_slice(&2i32.to_le_bytes()); // num_points
                                                    // Point 1
        shp.extend_from_slice(&1.0f64.to_le_bytes());
        shp.extend_from_slice(&2.0f64.to_le_bytes());
        // Point 2
        shp.extend_from_slice(&3.0f64.to_le_bytes());
        shp.extend_from_slice(&4.0f64.to_le_bytes());

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();
        assert_eq!(read_u32_le(&wkb, 1), WKB_MULTIPOINT);

        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp2, 0), SHP_MULTIPOINT);
    }

    #[test]
    fn wkb_linestring_to_shp_polyline() {
        // WKB LineString with 3 points → SHP Polyline (1 part)
        let mut wkb = Vec::new();
        wkb.push(WKB_LE);
        wkb.extend_from_slice(&WKB_LINESTRING.to_le_bytes());
        wkb.extend_from_slice(&3u32.to_le_bytes());
        for &(x, y) in &[(0.0f64, 0.0f64), (1.0, 1.0), (2.0, 0.0)] {
            wkb.extend_from_slice(&x.to_le_bytes());
            wkb.extend_from_slice(&y.to_le_bytes());
        }

        let shp = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp, 0), SHP_POLYLINE);
        assert_eq!(read_i32_le(&shp, 36), 1); // num_parts
        assert_eq!(read_i32_le(&shp, 40), 3); // num_points
    }

    #[test]
    fn wkb_polygon_to_shp_polygon() {
        // WKB Polygon with 1 ring → SHP Polygon
        let mut wkb = Vec::new();
        wkb.push(WKB_LE);
        wkb.extend_from_slice(&WKB_POLYGON.to_le_bytes());
        wkb.extend_from_slice(&1u32.to_le_bytes()); // 1 ring
        wkb.extend_from_slice(&4u32.to_le_bytes()); // 4 points
        for &(x, y) in &[(0.0f64, 0.0f64), (4.0, 0.0), (2.0, 3.0), (0.0, 0.0)] {
            wkb.extend_from_slice(&x.to_le_bytes());
            wkb.extend_from_slice(&y.to_le_bytes());
        }

        let shp = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp, 0), SHP_POLYGON);
        assert_eq!(read_i32_le(&shp, 36), 1); // num_parts
        assert_eq!(read_i32_le(&shp, 40), 4); // num_points
    }

    #[test]
    fn too_short_errors() {
        assert!(shp_to_wkb(&[]).is_err());
        assert!(shp_to_wkb(&[1, 0]).is_err());
        assert!(wkb_to_shp(&[]).is_err());
        assert!(wkb_to_shp(&[1, 0]).is_err());
    }

    #[test]
    fn unsupported_shp_type_errors() {
        let shp = 99i32.to_le_bytes();
        assert!(shp_to_wkb(&shp).is_err());
    }

    #[test]
    fn dataframe_spatial_roundtrip() {
        // Build SHP Point geometries
        let pt1 = {
            let mut v = Vec::new();
            v.extend_from_slice(&SHP_POINT.to_le_bytes());
            v.extend_from_slice(&(-73.9857f64).to_le_bytes());
            v.extend_from_slice(&40.7484f64.to_le_bytes());
            v
        };
        let pt2 = {
            let mut v = Vec::new();
            v.extend_from_slice(&SHP_POINT.to_le_bytes());
            v.extend_from_slice(&(2.2945f64).to_le_bytes());
            v.extend_from_slice(&48.8584f64.to_le_bytes());
            v
        };

        // Build a DataFrame with a SpatialObj column (raw SHP bytes)
        let fields = vec![
            FieldMeta {
                name: "id".into(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 0,
            },
            FieldMeta {
                name: "geom".into(),
                field_type: FieldType::SpatialObj,
                size: 0,
                scale: 0,
                offset: 5,
            },
        ];

        let id_col = Series::new("id".into(), &[1i32, 2]);
        let geom_col = Series::new(
            "geom".into(),
            vec![Some(pt1.as_slice()), Some(pt2.as_slice())],
        );
        let df = test_df(vec![id_col.into(), geom_col.into()]);

        // Convert SHP → WKB
        let df_wkb = convert_spatial_columns_to_wkb(df, &fields).unwrap();
        let wkb_col = df_wkb.column("geom").unwrap().binary().unwrap();
        let wkb0 = wkb_col.get(0).unwrap();
        assert_eq!(wkb0[0], WKB_LE);
        assert_eq!(read_u32_le(wkb0, 1), WKB_POINT);
        // X coordinate should be -73.9857
        let x = read_f64_le(wkb0, 5);
        assert!((x - (-73.9857)).abs() < 1e-10);

        // Convert WKB → SHP (round-trip)
        let df_shp = convert_spatial_columns_to_shp(&df_wkb, &fields).unwrap();
        let shp_col = df_shp.column("geom").unwrap().binary().unwrap();
        let shp0 = shp_col.get(0).unwrap();
        assert_eq!(read_i32_le(shp0, 0), SHP_POINT);
        assert_eq!(shp0, pt1.as_slice());
        let shp1 = shp_col.get(1).unwrap();
        assert_eq!(shp1, pt2.as_slice());
    }

    #[test]
    fn dataframe_null_spatial_values() {
        let fields = vec![FieldMeta {
            name: "geom".into(),
            field_type: FieldType::SpatialObj,
            size: 0,
            scale: 0,
            offset: 0,
        }];

        let geom_col = Series::new("geom".into(), vec![None::<Vec<u8>>, None]);
        let df = test_df(vec![geom_col.into()]);

        let df_wkb = convert_spatial_columns_to_wkb(df, &fields).unwrap();
        let col = df_wkb.column("geom").unwrap().binary().unwrap();
        assert!(col.get(0).is_none());
        assert!(col.get(1).is_none());
    }

    #[test]
    fn polyline_z_roundtrip() {
        // SHP PolylineZ with 1 part, 2 points, Z values, no M
        let mut shp = Vec::new();
        shp.extend_from_slice(&SHP_POLYLINE_Z.to_le_bytes());
        // bbox
        shp.extend_from_slice(&0.0f64.to_le_bytes());
        shp.extend_from_slice(&0.0f64.to_le_bytes());
        shp.extend_from_slice(&1.0f64.to_le_bytes());
        shp.extend_from_slice(&1.0f64.to_le_bytes());
        shp.extend_from_slice(&1i32.to_le_bytes()); // num_parts
        shp.extend_from_slice(&2i32.to_le_bytes()); // num_points
        shp.extend_from_slice(&0i32.to_le_bytes()); // part[0]
                                                    // XY
        shp.extend_from_slice(&0.0f64.to_le_bytes());
        shp.extend_from_slice(&0.0f64.to_le_bytes());
        shp.extend_from_slice(&1.0f64.to_le_bytes());
        shp.extend_from_slice(&1.0f64.to_le_bytes());
        // Z range
        shp.extend_from_slice(&10.0f64.to_le_bytes());
        shp.extend_from_slice(&20.0f64.to_le_bytes());
        // Z values
        shp.extend_from_slice(&10.0f64.to_le_bytes());
        shp.extend_from_slice(&20.0f64.to_le_bytes());

        let wkb = shp_to_wkb(&shp).unwrap().unwrap();
        assert_eq!(read_u32_le(&wkb, 1), WKB_MULTILINESTRING + WKB_Z);

        let shp2 = wkb_to_shp(&wkb).unwrap();
        assert_eq!(read_i32_le(&shp2, 0), SHP_POLYLINE_Z);
    }

    #[test]
    fn spatial_column_names_filters_correctly() {
        let fields = vec![
            FieldMeta {
                name: "id".into(),
                field_type: FieldType::Int32,
                size: 4,
                scale: 0,
                offset: 0,
            },
            FieldMeta {
                name: "geom".into(),
                field_type: FieldType::SpatialObj,
                size: 0,
                scale: 0,
                offset: 5,
            },
            FieldMeta {
                name: "name".into(),
                field_type: FieldType::VWString,
                size: 256,
                scale: 0,
                offset: 9,
            },
            FieldMeta {
                name: "shape".into(),
                field_type: FieldType::SpatialObj,
                size: 0,
                scale: 0,
                offset: 13,
            },
        ];

        let names = spatial_column_names(&fields);
        assert_eq!(names, vec!["geom", "shape"]);
    }

    #[test]
    fn spatial_column_names_empty_when_no_spatial() {
        let fields = vec![FieldMeta {
            name: "id".into(),
            field_type: FieldType::Int32,
            size: 4,
            scale: 0,
            offset: 0,
        }];

        let names = spatial_column_names(&fields);
        assert!(names.is_empty());
    }

    #[test]
    fn spatial_mode_geoarrow_produces_wkb() {
        // GeoArrow mode should produce the same WKB output as Wkb mode
        let pt = {
            let mut v = Vec::new();
            v.extend_from_slice(&SHP_POINT.to_le_bytes());
            v.extend_from_slice(&(-73.9857f64).to_le_bytes());
            v.extend_from_slice(&40.7484f64.to_le_bytes());
            v
        };

        let fields = vec![FieldMeta {
            name: "geom".into(),
            field_type: FieldType::SpatialObj,
            size: 0,
            scale: 0,
            offset: 0,
        }];

        let geom_col = Series::new("geom".into(), vec![Some(pt.as_slice())]);
        let df = test_df(vec![geom_col.into()]);

        // Both Wkb and GeoArrow should produce the same DataFrame
        let df_wkb = convert_spatial_columns_to_wkb(df.clone(), &fields).unwrap();
        let df_geo = convert_spatial_columns_to_wkb(df, &fields).unwrap();

        let wkb_col = df_wkb.column("geom").unwrap().binary().unwrap();
        let geo_col = df_geo.column("geom").unwrap().binary().unwrap();
        assert_eq!(wkb_col.get(0), geo_col.get(0));
    }
}
