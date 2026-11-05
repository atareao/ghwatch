//! Puntuación base de CVSS v3 a partir de su vector.
//!
//! OSV publica la severidad como el **vector** CVSS (por ejemplo
//! `CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H`), no como un número. Para
//! completar el `cvss` de una alerta hace falta la puntuación base, así que este
//! módulo implementa la fórmula oficial de CVSS v3.0/v3.1.
//!
//! Es una función **pura**: sin IO, sin red, sin estado. Si el vector está
//! incompleto o trae un valor no reconocido, devuelve `None` en lugar de
//! aproximar.

/// Puntuación base de un vector CVSS v3.0/v3.1; `None` si no es interpretable.
///
/// Acepta también una puntuación numérica directa (algunos registros publican el
/// número en lugar del vector), de modo que el enriquecimiento no descarta un
/// dato válido por su forma. Un número no finito (`NaN`, `inf`) o fuera del
/// rango `[0, 10]` se descarta: contaminaría la priorización.
#[must_use]
pub fn parse_score(score: &str) -> Option<f64> {
    let score = score.trim();
    if let Ok(numero) = score.parse::<f64>() {
        return puntuacion_valida(numero);
    }
    if score.to_ascii_uppercase().starts_with("CVSS:") {
        return v3_base_score(score);
    }
    None
}

/// Acepta una puntuación solo si es finita y está dentro de `[0, 10]`.
fn puntuacion_valida(valor: f64) -> Option<f64> {
    (valor.is_finite() && (0.0..=10.0).contains(&valor)).then_some(valor)
}

/// Puntuación base de un vector CVSS v3.0/v3.1.
///
/// Devuelve `None` si falta alguna métrica obligatoria o trae un valor fuera del
/// conjunto cerrado.
#[must_use]
pub fn v3_base_score(vector: &str) -> Option<f64> {
    let scope_changed = match metrica(vector, "S")? {
        'U' => false,
        'C' => true,
        _ => return None,
    };

    let av = match metrica(vector, "AV")? {
        'N' => 0.85,
        'A' => 0.62,
        'L' => 0.55,
        'P' => 0.20,
        _ => return None,
    };
    let ac = match metrica(vector, "AC")? {
        'L' => 0.77,
        'H' => 0.44,
        _ => return None,
    };
    let ui = match metrica(vector, "UI")? {
        'N' => 0.85,
        'R' => 0.62,
        _ => return None,
    };
    let pr = match metrica(vector, "PR")? {
        'N' => 0.85,
        'L' if scope_changed => 0.68,
        'L' => 0.62,
        'H' if scope_changed => 0.50,
        'H' => 0.27,
        _ => return None,
    };

    let c = impacto(metrica(vector, "C")?)?;
    let i = impacto(metrica(vector, "I")?)?;
    let a = impacto(metrica(vector, "A")?)?;

    let iss = 1.0 - (1.0 - c) * (1.0 - i) * (1.0 - a);
    let impacto_total = if scope_changed {
        7.52 * (iss - 0.029) - 3.25 * (iss - 0.02).powi(15)
    } else {
        6.42 * iss
    };

    if impacto_total <= 0.0 {
        return Some(0.0);
    }

    let explotabilidad = 8.22 * av * ac * pr * ui;
    let bruto = if scope_changed {
        (1.08 * (impacto_total + explotabilidad)).min(10.0)
    } else {
        (impacto_total + explotabilidad).min(10.0)
    };

    Some(redondear_hacia_arriba(bruto))
}

/// Valor de una métrica del vector (`AV`, `AC`, `PR`, `UI`, `S`, `C`, `I`, `A`).
fn metrica(vector: &str, clave: &str) -> Option<char> {
    vector.split('/').find_map(|parte| {
        let (nombre, valor) = parte.split_once(':')?;
        if nombre == clave {
            valor.chars().next()
        } else {
            None
        }
    })
}

/// Contribución de una métrica de impacto (`C`, `I`, `A`).
fn impacto(valor: char) -> Option<f64> {
    match valor {
        'H' => Some(0.56),
        'L' => Some(0.22),
        'N' => Some(0.0),
        _ => None,
    }
}

/// Redondeo oficial de CVSS v3: al alza al primer decimal.
fn redondear_hacia_arriba(valor: f64) -> f64 {
    let entero = (valor * 100_000.0).round() as i64;
    if entero % 10_000 == 0 {
        entero as f64 / 100_000.0
    } else {
        ((entero / 10_000) + 1) as f64 / 10.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un vector crítico conocido da 9.8.
    #[test]
    fn un_vector_critico_da_9_8() {
        assert_eq!(
            parse_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            Some(9.8)
        );
    }

    /// Un vector sin impacto da 0.0.
    #[test]
    fn un_vector_sin_impacto_da_cero() {
        assert_eq!(
            parse_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:N"),
            Some(0.0)
        );
    }

    /// Un vector con el alcance cambiado satura en 10.0.
    #[test]
    fn un_vector_de_alcance_cambiado_satura() {
        assert_eq!(
            parse_score("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"),
            Some(10.0)
        );
    }

    /// Una puntuación numérica directa se acepta tal cual.
    #[test]
    fn una_puntuacion_numerica_se_acepta() {
        assert_eq!(parse_score("7.5"), Some(7.5));
        assert_eq!(parse_score(" 0 "), Some(0.0));
    }

    /// M1 — Una puntuación no finita o fuera del rango `[0, 10]` se descarta en
    /// lugar de contaminar la priorización.
    #[test]
    fn una_puntuacion_no_finita_o_fuera_de_rango_se_descarta() {
        assert_eq!(parse_score("NaN"), None);
        assert_eq!(parse_score("inf"), None);
        assert_eq!(parse_score("-inf"), None);
        assert_eq!(parse_score("-1.0"), None);
        assert_eq!(parse_score("10.1"), None);
        assert_eq!(
            parse_score("10"),
            Some(10.0),
            "el límite superior es válido"
        );
        assert_eq!(parse_score("0"), Some(0.0), "el límite inferior es válido");
    }

    /// Un vector incompleto o con un valor no reconocido no se interpreta.
    #[test]
    fn un_vector_invalido_no_se_interpreta() {
        assert_eq!(parse_score("CVSS:3.1/AV:N/AC:L"), None);
        assert_eq!(
            parse_score("CVSS:3.1/AV:Z/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"),
            None
        );
        assert_eq!(parse_score("no es un vector"), None);
    }
}
