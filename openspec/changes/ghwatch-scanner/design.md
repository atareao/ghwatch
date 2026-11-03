# Design: ghwatch-scanner

## Decisiones

### Dependabot como fuente primaria; SBOM descartado

El caso medido lo decide: `reto-python` devuelve 112 alertas de Dependabot, pero el dependency-graph de GitHub solo ve 33 paquetes, porque solo resuelve los manifiestos de raíz y no los que viven en subcarpetas como `reto-11/joselo/requirements.txt`. Dependabot sí los analiza. Por tanto, la fuente primaria es `GET /repos/{r}/dependabot/alerts`, y el SBOM no se usa como fuente.

### Paginación obligatoria

Con `per_page=100`, `reto-python` devuelve 100 de sus 112 alertas. Cualquier recolección que se conforme con la primera página pierde alertas de forma silenciosa. La paginación se sigue siempre por el encabezado `Link` con `rel="next"` hasta agotarlo, sin excepciones por tamaño de página.

### Rate limit

433 repositorios × 3 endpoints ≈ 1.100 llamadas frente a una cuota core de 5.000 req/h. El escaneo cabe holgadamente, pero no si se abusa de la concurrencia. La concurrencia se acota a 6; se leen `X-RateLimit-Remaining` y `X-RateLimit-Reset` para pausar antes de agotar la cuota; y ante un `403` secundario se respeta `Retry-After` con retroceso exponencial hasta un máximo de reintentos, tras el cual se devuelve un error tipado.

### Recorte por bytes UTF-8

El límite de 4096 bytes de Valet es de bytes, no de caracteres. Recortar por número de caracteres rompería emojis y acentos y produciría UTF-8 inválido. El recorte se hace por límite de bytes, avanzando hasta la última frontera de carácter válida, de modo que el `message` siempre sea UTF-8 válido y no supere 4096 bytes. El metadato sigue la misma disciplina contra su límite de 8192 bytes.

### Modelo de deriva

La deriva clasifica cada alerta en una categoría cerrada: `new`, `still_open`, `resolved`, `reopened`, `severity_up`. `reopened` no es `new` porque la alerta ya existía y tiene `first_seen_run` previo: agruparla con las nuevas inflaría el recuento de novedades y ocultaría que algo ya corregido ha vuelto. La distinción permite que la notificación y el neto reflejen reaperturas sin contarlas como descubrimientos.

### sqlx con consultas en tiempo de ejecución

Se usa sqlx con SQLite y consultas resueltas en tiempo de ejecución (`runtime-tokio`, sin macros que verifiquen contra la base en compilación). Así el build no depende de `DATABASE_URL` ni de una base preparada, lo que mantiene la CI y el desarrollo libres de un paso de preparación de esquema.

### Sin red en los tests

GitHub, OSV y Valet se simulan con `wiremock`. Los tests no realizan tráfico de red real: los casos de paginación, `401`, `403` con `Retry-After`, `404` `no analysis found`, los lotes de OSV y las respuestas de Valet se sirven desde mocks. Esto hace la suite determinista, rápida y ejecutable sin credenciales.

### Valet no se modifica

La integración con Valet es un cliente HTTP sobre su API de ingesta ya existente, documentada en `/data/rust/valet/docs/api-notifications.md`. `ghwatch` no cambia el otro proyecto: solo hace `POST {VALET_BASE_URL}/api/notifications` esperando `201`. La notificación es best-effort y nunca condiciona el resultado del escaneo ni la escritura del informe.
