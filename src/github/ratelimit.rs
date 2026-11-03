//! Control del límite de peticiones de la API de GitHub.
//!
//! Todas las funciones de este módulo son **puras**: reciben los valores ya
//! extraídos de las cabeceras y deciden qué hacer, sin IO ni relojes ocultos.
//! El cliente es quien lee las cabeceras y quien duerme; aquí solo vive la
//! aritmética, que así se prueba directamente.

use std::time::Duration;

/// Espera conservadora cuando la cuota está por debajo del umbral pero la
/// cabecera `X-RateLimit-Reset` no permite calcular cuándo se restablece.
pub const FALLBACK_PAUSE: Duration = Duration::from_secs(60);

/// Techo del retroceso exponencial entre reintentos.
const MAX_BACKOFF_SECS: u64 = 60;

/// Estado del límite de peticiones tal y como queda tras una respuesta.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimitState {
    /// Peticiones restantes, según `X-RateLimit-Remaining`.
    pub remaining: Option<u64>,
    /// Tiempo que falta hasta el restablecimiento, según `X-RateLimit-Reset`.
    pub reset: Option<Duration>,
}

/// Interpreta `X-RateLimit-Remaining` y `X-RateLimit-Reset`.
///
/// `reset` llega como instante absoluto (segundos desde la época UNIX) y se
/// convierte en una espera relativa restando `now_epoch`. El instante actual se
/// pasa explícitamente para mantener la función pura y comprobable.
///
/// Los valores no numéricos se ignoran (`None`) en lugar de provocar un error:
/// una cabecera ilegible no debe romper el escaneo.
#[must_use]
pub fn rate_limit_state(
    remaining: Option<&str>,
    reset: Option<&str>,
    now_epoch: u64,
) -> RateLimitState {
    let remaining = remaining.and_then(|valor| valor.trim().parse::<u64>().ok());
    let reset = reset
        .and_then(|valor| valor.trim().parse::<u64>().ok())
        .map(|reset_epoch| Duration::from_secs(reset_epoch.saturating_sub(now_epoch)));

    RateLimitState { remaining, reset }
}

/// Decide cuánto esperar antes de la siguiente petición.
///
/// Devuelve `Some(espera)` **solo** cuando la cuota restante está **por debajo**
/// del umbral: la espera es hasta `reset` y, si no se conoce, [`FALLBACK_PAUSE`].
///
/// Sin una lectura de `X-RateLimit-Remaining` no hay pausa: `remaining == None`
/// devuelve `None` aunque se conozca `reset`. Decisión deliberada: [`FALLBACK_PAUSE`]
/// es una espera conservadora para un caso en el que **sí** consta que la cuota
/// está baja pero no cuándo se repone; inventar una pausa cuando ni siquiera se
/// ha leído la cuota restante penalizaría el escaneo sin ninguna evidencia.
#[must_use]
pub fn should_pause_before_request(state: &RateLimitState, threshold: u64) -> Option<Duration> {
    let remaining = state.remaining?;
    if remaining >= threshold {
        return None;
    }
    Some(state.reset.unwrap_or(FALLBACK_PAUSE))
}

/// Extrae los segundos de la cabecera `Retry-After`.
///
/// GitHub envía esta cabecera como número de segundos. Un valor ausente o no
/// numérico devuelve `None` (el cliente aplicará retroceso exponencial).
#[must_use]
pub fn parse_retry_after(value: Option<&str>) -> Option<Duration> {
    value
        .and_then(|valor| valor.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// Retroceso exponencial entre reintentos: 1 s, 2 s, 4 s, 8 s… con un techo.
///
/// `attempt` es el número de reintentos ya realizados (0 para el primero).
#[must_use]
pub fn backoff_delay(attempt: u32) -> Duration {
    let segundos = 1u64 << attempt.min(6);
    Duration::from_secs(segundos.min(MAX_BACKOFF_SECS))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Remaining` y `Reset` se interpretan y `Reset` se convierte en espera.
    #[test]
    fn rate_limit_state_interpreta_las_cabeceras() {
        let estado = rate_limit_state(Some("5"), Some("1000"), 940);
        assert_eq!(
            estado,
            RateLimitState {
                remaining: Some(5),
                reset: Some(Duration::from_secs(60)),
            }
        );
    }

    /// Un `Reset` ya pasado no produce esperas negativas.
    #[test]
    fn rate_limit_state_reset_pasado_es_cero() {
        let estado = rate_limit_state(Some("0"), Some("1000"), 1200);
        assert_eq!(estado.reset, Some(Duration::ZERO));
    }

    /// Sin cabeceras no hay estado.
    #[test]
    fn rate_limit_state_sin_cabeceras() {
        assert_eq!(
            rate_limit_state(None, None, 1_000),
            RateLimitState::default()
        );
    }

    /// Un valor no numérico se ignora en lugar de romper.
    #[test]
    fn rate_limit_state_valores_no_numericos() {
        assert_eq!(
            rate_limit_state(Some("many"), Some("later"), 0),
            RateLimitState::default()
        );
    }

    /// Por encima del umbral no se pausa.
    #[test]
    fn should_pause_por_encima_del_umbral() {
        let estado = RateLimitState {
            remaining: Some(500),
            reset: Some(Duration::from_secs(60)),
        };
        assert_eq!(should_pause_before_request(&estado, 100), None);
    }

    /// Justo en el umbral tampoco se pausa.
    #[test]
    fn should_pause_en_el_umbral_no_pausa() {
        let estado = RateLimitState {
            remaining: Some(100),
            reset: Some(Duration::from_secs(60)),
        };
        assert_eq!(should_pause_before_request(&estado, 100), None);
    }

    /// Por debajo del umbral se espera hasta el restablecimiento.
    #[test]
    fn should_pause_por_debajo_del_umbral() {
        let estado = RateLimitState {
            remaining: Some(5),
            reset: Some(Duration::from_secs(42)),
        };
        assert_eq!(
            should_pause_before_request(&estado, 100),
            Some(Duration::from_secs(42))
        );
    }

    /// Por debajo del umbral sin `Reset` se usa la espera conservadora.
    #[test]
    fn should_pause_por_debajo_sin_reset_usa_fallback() {
        let estado = RateLimitState {
            remaining: Some(5),
            reset: None,
        };
        assert_eq!(
            should_pause_before_request(&estado, 100),
            Some(FALLBACK_PAUSE)
        );
    }

    /// Sin `Remaining` legible **no** se pausa aunque haya `Reset`: no se aplica
    /// la espera conservadora sin evidencia de que la cuota esté baja.
    #[test]
    fn should_pause_sin_remaining_con_reset_no_pausa() {
        let estado = RateLimitState {
            remaining: None,
            reset: Some(Duration::from_secs(10)),
        };
        assert_eq!(should_pause_before_request(&estado, 100), None);
    }

    /// Sin cabeceras no se pausa.
    #[test]
    fn should_pause_sin_cabeceras() {
        assert_eq!(
            should_pause_before_request(&RateLimitState::default(), 100),
            None
        );
    }

    /// `Retry-After` numérico se interpreta en segundos.
    #[test]
    fn parse_retry_after_numerico() {
        assert_eq!(
            parse_retry_after(Some(" 30 ")),
            Some(Duration::from_secs(30))
        );
    }

    /// `Retry-After` ausente o no numérico devuelve `None`.
    #[test]
    fn parse_retry_after_invalido() {
        assert_eq!(parse_retry_after(None), None);
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT")),
            None
        );
    }

    /// El retroceso crece de forma exponencial y tiene techo.
    #[test]
    fn backoff_delay_crece_y_se_estabiliza() {
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(2), Duration::from_secs(4));
        assert_eq!(backoff_delay(10), Duration::from_secs(MAX_BACKOFF_SECS));
    }
}
