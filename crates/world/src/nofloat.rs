//! Integer math that gives the same results on every machine
//! (`lib/nofloat.py`), and the Python integer semantics the simulation
//! relies on.
//!
//! Coordinates, hit points, speeds and the like are integers scaled by
//! [`PRECISION`]: 1.5 hit points is stored as 1500.

pub const PRECISION: i64 = 1000;

/// Python's `a // b`: the quotient rounded toward negative infinity.
pub fn floordiv(a: i64, b: i64) -> i64 {
    a.div_euclid(b) - i64::from(b < 0 && a.rem_euclid(b) != 0)
}

/// Python's `a % b`: the remainder takes the sign of the divisor.
pub fn pymod(a: i64, b: i64) -> i64 {
    a - b * floordiv(a, b)
}

/// The error of [`to_int`] and [`py_float`]: the text is not a number
/// Python's `float()` accepts, or the result is not a finite integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotANumber;

/// Python's `float(s)`: surrounding whitespace is ignored, single underscores
/// may separate digits, and `inf` and `nan` are accepted.
pub fn py_float(s: &str) -> Result<f64, NotANumber> {
    let t = s.trim_matches(|c: char| c.is_whitespace());
    if t.is_empty() {
        return Err(NotANumber);
    }
    let cleaned;
    let t = if t.contains('_') {
        let b = t.as_bytes();
        for (i, &c) in b.iter().enumerate() {
            if c == b'_' {
                let ok = i > 0
                    && i + 1 < b.len()
                    && b[i - 1].is_ascii_digit()
                    && b[i + 1].is_ascii_digit();
                if !ok {
                    return Err(NotANumber);
                }
            }
        }
        cleaned = t.replace('_', "");
        cleaned.as_str()
    } else {
        t
    };
    // Rust accepts the same spellings as Python ("1.", ".5", "1e3", "inf",
    // "infinity", "nan", with a sign), and rounds correctly like Python.
    t.parse::<f64>().map_err(|_| NotANumber)
}

/// Converts a decimal written in a definition file to an integer with
/// [`PRECISION`]: `int(float(s) * PRECISION)`.
///
/// The value goes through a binary float, as in the original, so "1.005"
/// gives 1004. Every machine computes the same IEEE product, so the result
/// is still deterministic.
pub fn to_int(s: &str) -> Result<i64, NotANumber> {
    float_to_int(py_float(s)? * PRECISION as f64)
}

/// Python's `int(f)` for a float: truncation toward zero.
pub fn float_to_int(f: f64) -> Result<i64, NotANumber> {
    if !f.is_finite() || f.abs() >= 9.2e18 {
        return Err(NotANumber);
    }
    Ok(f.trunc() as i64)
}

/// Python's `int(s)` for a decimal integer written as text.
pub fn py_int(s: &str) -> Result<i64, NotANumber> {
    let t = s.trim_matches(|c: char| c.is_whitespace());
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') {
        return Err(NotANumber);
    }
    let mut value: i64 = 0;
    let mut previous_underscore = false;
    for c in digits.chars() {
        if c == '_' {
            if previous_underscore {
                return Err(NotANumber);
            }
            previous_underscore = true;
            continue;
        }
        previous_underscore = false;
        let d = c.to_digit(10).ok_or(NotANumber)?;
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(i64::from(d)))
            .ok_or(NotANumber)?;
    }
    Ok(if neg { -value } else { value })
}

/// The cosine of an angle in degrees, times 1000, from a fixed table.
pub fn int_cos_1000(angle: i64) -> i64 {
    tables::COS_TABLE[pymod(angle, 360) as usize]
}

/// The sine of an angle in degrees, times 1000, from a fixed table.
pub fn int_sin_1000(angle: i64) -> i64 {
    tables::SIN_TABLE[pymod(angle, 360) as usize]
}

pub fn square_of_distance(x1: i64, y1: i64, x2: i64, y2: i64) -> i64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    dx * dx + dy * dy
}

pub fn int_distance(x1: i64, y1: i64, x2: i64, y2: i64) -> i64 {
    int_sqrt(square_of_distance(x1, y1, x2, y2))
}

/// The integer square root, as `int(math.sqrt(x))` then corrected.
///
/// The correction stops at `(r + 1)² < x`, not `<=`, like the original.
pub fn int_sqrt(x: i64) -> i64 {
    let mut r = (x as f64).sqrt() as i64;
    while r * r > x {
        r -= 1;
    }
    while (r + 1) * (r + 1) < x {
        r += 1;
    }
    r
}

/// The angle in degrees between the x axis and the line from the first
/// point to the second, from a table of arc cosines in steps of 0.01.
pub fn int_angle(x1: i64, y1: i64, x2: i64, y2: i64) -> i64 {
    let d = int_distance(x1, y1, x2, y2);
    if d == 0 {
        return 0;
    }
    let c = floordiv((x2 - x1) * 100, d);
    let ac = tables::ACOS_TABLE[(c + 100) as usize];
    if y2 - y1 > 0 { ac } else { -ac }
}

mod tables {
    pub(crate) const COS_TABLE: [i64; 360] = [
        1000, 999, 999, 998, 997, 996, 994, 992, 990, 987, 984, 981, 978, 974, 970, 965, 961, 956,
        951, 945, 939, 933, 927, 920, 913, 906, 898, 891, 882, 874, 866, 857, 848, 838, 829, 819,
        809, 798, 788, 777, 766, 754, 743, 731, 719, 707, 694, 681, 669, 656, 642, 629, 615, 601,
        587, 573, 559, 544, 529, 515, 500, 484, 469, 453, 438, 422, 406, 390, 374, 358, 342, 325,
        309, 292, 275, 258, 241, 224, 207, 190, 173, 156, 139, 121, 104, 87, 69, 52, 34, 17, 0,
        -17, -34, -52, -69, -87, -104, -121, -139, -156, -173, -190, -207, -224, -241, -258, -275,
        -292, -309, -325, -342, -358, -374, -390, -406, -422, -438, -453, -469, -484, -499, -515,
        -529, -544, -559, -573, -587, -601, -615, -629, -642, -656, -669, -681, -694, -707, -719,
        -731, -743, -754, -766, -777, -788, -798, -809, -819, -829, -838, -848, -857, -866, -874,
        -882, -891, -898, -906, -913, -920, -927, -933, -939, -945, -951, -956, -961, -965, -970,
        -974, -978, -981, -984, -987, -990, -992, -994, -996, -997, -998, -999, -999, -1000, -999,
        -999, -998, -997, -996, -994, -992, -990, -987, -984, -981, -978, -974, -970, -965, -961,
        -956, -951, -945, -939, -933, -927, -920, -913, -906, -898, -891, -882, -874, -866, -857,
        -848, -838, -829, -819, -809, -798, -788, -777, -766, -754, -743, -731, -719, -707, -694,
        -681, -669, -656, -642, -629, -615, -601, -587, -573, -559, -544, -529, -515, -500, -484,
        -469, -453, -438, -422, -406, -390, -374, -358, -342, -325, -309, -292, -275, -258, -241,
        -224, -207, -190, -173, -156, -139, -121, -104, -87, -69, -52, -34, -17, 0, 17, 34, 52, 69,
        87, 104, 121, 139, 156, 173, 190, 207, 224, 241, 258, 275, 292, 309, 325, 342, 358, 374,
        390, 406, 422, 438, 453, 469, 484, 500, 515, 529, 544, 559, 573, 587, 601, 615, 629, 642,
        656, 669, 681, 694, 707, 719, 731, 743, 754, 766, 777, 788, 798, 809, 819, 829, 838, 848,
        857, 866, 874, 882, 891, 898, 906, 913, 920, 927, 933, 939, 945, 951, 956, 961, 965, 970,
        974, 978, 981, 984, 987, 990, 992, 994, 996, 997, 998, 999, 999,
    ];
    pub(crate) const SIN_TABLE: [i64; 360] = [
        0, 17, 34, 52, 69, 87, 104, 121, 139, 156, 173, 190, 207, 224, 241, 258, 275, 292, 309,
        325, 342, 358, 374, 390, 406, 422, 438, 453, 469, 484, 499, 515, 529, 544, 559, 573, 587,
        601, 615, 629, 642, 656, 669, 681, 694, 707, 719, 731, 743, 754, 766, 777, 788, 798, 809,
        819, 829, 838, 848, 857, 866, 874, 882, 891, 898, 906, 913, 920, 927, 933, 939, 945, 951,
        956, 961, 965, 970, 974, 978, 981, 984, 987, 990, 992, 994, 996, 997, 998, 999, 999, 1000,
        999, 999, 998, 997, 996, 994, 992, 990, 987, 984, 981, 978, 974, 970, 965, 961, 956, 951,
        945, 939, 933, 927, 920, 913, 906, 898, 891, 882, 874, 866, 857, 848, 838, 829, 819, 809,
        798, 788, 777, 766, 754, 743, 731, 719, 707, 694, 681, 669, 656, 642, 629, 615, 601, 587,
        573, 559, 544, 529, 515, 499, 484, 469, 453, 438, 422, 406, 390, 374, 358, 342, 325, 309,
        292, 275, 258, 241, 224, 207, 190, 173, 156, 139, 121, 104, 87, 69, 52, 34, 17, 0, -17,
        -34, -52, -69, -87, -104, -121, -139, -156, -173, -190, -207, -224, -241, -258, -275, -292,
        -309, -325, -342, -358, -374, -390, -406, -422, -438, -453, -469, -484, -500, -515, -529,
        -544, -559, -573, -587, -601, -615, -629, -642, -656, -669, -681, -694, -707, -719, -731,
        -743, -754, -766, -777, -788, -798, -809, -819, -829, -838, -848, -857, -866, -874, -882,
        -891, -898, -906, -913, -920, -927, -933, -939, -945, -951, -956, -961, -965, -970, -974,
        -978, -981, -984, -987, -990, -992, -994, -996, -997, -998, -999, -999, -1000, -999, -999,
        -998, -997, -996, -994, -992, -990, -987, -984, -981, -978, -974, -970, -965, -961, -956,
        -951, -945, -939, -933, -927, -920, -913, -906, -898, -891, -882, -874, -866, -857, -848,
        -838, -829, -819, -809, -798, -788, -777, -766, -754, -743, -731, -719, -707, -694, -681,
        -669, -656, -642, -629, -615, -601, -587, -573, -559, -544, -529, -515, -500, -484, -469,
        -453, -438, -422, -406, -390, -374, -358, -342, -325, -309, -292, -275, -258, -241, -224,
        -207, -190, -173, -156, -139, -121, -104, -87, -69, -52, -34, -17,
    ];
    /// Indexed by `c + 100` for `c` in `-100..=100`.
    pub(crate) const ACOS_TABLE: [i64; 201] = [
        180, 171, 168, 165, 163, 161, 160, 158, 156, 155, 154, 152, 151, 150, 149, 148, 147, 146,
        145, 144, 143, 142, 141, 140, 139, 138, 137, 136, 136, 135, 134, 133, 132, 132, 131, 130,
        129, 129, 128, 127, 126, 126, 125, 124, 124, 123, 122, 122, 121, 120, 120, 119, 118, 118,
        117, 116, 116, 115, 114, 114, 113, 112, 112, 111, 111, 110, 109, 109, 108, 108, 107, 106,
        106, 105, 105, 104, 103, 103, 102, 102, 101, 100, 100, 99, 99, 98, 98, 97, 96, 96, 95, 95,
        94, 94, 93, 92, 92, 91, 91, 90, 90, 89, 88, 88, 87, 87, 86, 85, 85, 84, 84, 83, 83, 82, 81,
        81, 80, 80, 79, 79, 78, 77, 77, 76, 76, 75, 74, 74, 73, 73, 72, 71, 71, 70, 70, 69, 68, 68,
        67, 67, 66, 65, 65, 64, 63, 63, 62, 61, 61, 60, 59, 59, 58, 57, 57, 56, 55, 55, 54, 53, 53,
        52, 51, 50, 50, 49, 48, 47, 47, 46, 45, 44, 43, 43, 42, 41, 40, 39, 38, 37, 36, 35, 34, 33,
        32, 31, 30, 29, 28, 27, 25, 24, 23, 21, 19, 18, 16, 14, 11, 8, 0,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_division_floors_and_the_remainder_follows_the_divisor() {
        assert_eq!(floordiv(7, 2), 3);
        assert_eq!(floordiv(-7, 2), -4);
        assert_eq!(floordiv(7, -2), -4);
        assert_eq!(floordiv(-7, -2), 3);
        assert_eq!(pymod(-1, 360), 359);
        assert_eq!(pymod(7, -2), -1);
    }

    #[test]
    fn to_int_matches_python_including_binary_float_rounding() {
        // Expected values printed by CPython 3.11: int(float(s) * 1000).
        let cases = [
            ("0.29", 290),
            ("0.57", 570),
            ("1.1", 1100),
            ("133.3", 133300),
            ("266.7", 266700),
            ("0.17", 170),
            (".01", 10),
            ("2147483", 2147483000),
            ("-0.5", -500),
            ("1e2", 100000),
            (" 3 ", 3000),
            ("1_5", 15000),
            ("0.015", 15),
            ("1.005", 1004),
            ("4.35", 4350),
        ];
        for (s, expected) in cases {
            assert_eq!(to_int(s), Ok(expected), "to_int({s:?})");
        }
        assert_eq!(to_int("a1"), Err(NotANumber));
        assert_eq!(to_int("inf"), Err(NotANumber));
        assert_eq!(to_int("nan"), Err(NotANumber));
        assert_eq!(to_int("1__0"), Err(NotANumber));
        assert_eq!(to_int(""), Err(NotANumber));
    }

    #[test]
    fn int_sqrt_matches_python() {
        assert_eq!(int_sqrt(0), 0);
        assert_eq!(int_sqrt(15), 3);
        assert_eq!(int_sqrt(16), 4);
        assert_eq!(int_sqrt(10_i64.pow(16) + 1), 100_000_000);
        assert_eq!(int_sqrt(2_i64.pow(53) + 1), 94_906_265);
    }

    #[test]
    fn angles_come_from_the_tables() {
        assert_eq!(int_cos_1000(0), 1000);
        assert_eq!(int_cos_1000(-90), 0);
        assert_eq!(int_sin_1000(450), 1000);
        assert_eq!(int_angle(0, 0, 0, 0), 0);
        assert_eq!(int_angle(0, 0, 10, 0), 0);
        assert_eq!(int_angle(0, 0, 0, 10), 90);
        assert_eq!(int_angle(0, 0, 0, -10), -90);
        assert_eq!(int_angle(0, 0, -10, 0), -180);
        // CPython 3.11 with the original module gives 44 and -180.
        assert_eq!(int_angle(0, 0, 10, 10), 44);
        assert_eq!(int_angle(3, 4, -7, 1), -180);
    }

    #[test]
    fn py_int_accepts_what_python_int_accepts() {
        assert_eq!(py_int("12"), Ok(12));
        assert_eq!(py_int(" -3 "), Ok(-3));
        assert_eq!(py_int("1_000"), Ok(1000));
        assert_eq!(py_int("1.5"), Err(NotANumber));
        assert_eq!(py_int(""), Err(NotANumber));
    }
}
