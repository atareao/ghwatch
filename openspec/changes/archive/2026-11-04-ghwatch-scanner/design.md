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

## Hallazgos del mundo real (Fase 2)

### La paginación es por cursor y cambia el prefijo de la ruta

Medido sobre `GET /repos/atareao/reto-python/dependabot/alerts?state=open&per_page=100` (112 avisos en 2 páginas), el `Link` de la segunda página **no** es la misma ruta con `page=2`:

```
Link: <https://api.github.com/repositories/459293568/dependabot/alerts?state=open&per_page=100&after=Y3Vyc29yOnYyOpPPAAABjxPlouHLP_fn_1g6U7nPAAAAAUdOwhk%3D>; rel="next"
```

Cambia el prefijo (`/repos/{o}/{r}/...` → `/repositories/{id}/...`) y añade un cursor opaco `after=`. Por eso el cliente sigue la URL del `Link` **tal cual**, sin recomponerla: recomponer a partir de la ruta original pierde la página siguiente o entra en bucle. Esta es exactamente la trampa que `wiremock` no detectaba (un mock con `query_param` casa aunque sobren parámetros), así que el test de paginación se hizo con forma de cursor y con la página 2 montada **solo** bajo la ruta del cursor.

### Un `403` puede significar «capacidad deshabilitada», no un fallo

De 433 repositorios, 417 responden `200` a `dependabot/alerts` y 16 responden `403` con este cuerpo literal:

```json
{"message":"Dependabot alerts are disabled for this repository.","documentation_url":"https://docs.github.com/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository","status":"403"}
```

El cliente lo clasifica como `GithubError::Disabled` (por el cuerpo, con `is_disabled_body`), distinto de un `403` de límite (`RateLimited`, que trae `Retry-After`) y de un `403` genérico (`Http{403}`). Sonda alternativa documentada para quien no quiera depender del texto: `GET /repos/{r}/vulnerability-alerts` responde `204` si los avisos están activados y `404` si están deshabilitados.

### Cuota observada

La respuesta real trae `X-Ratelimit-Limit: 5000`, `X-Ratelimit-Remaining: 4620` y `X-Ratelimit-Reset: 1793690335`. El cliente lee esas cabeceras y pausa de forma preventiva solo cuando `Remaining` cae por debajo del umbral; si no hay una lectura de `Remaining`, no pausa.

### Modelo de deriva

La deriva clasifica cada alerta en una categoría cerrada: `new`, `still_open`, `resolved`, `reopened`, `severity_up`. `reopened` no es `new` porque la alerta ya existía y tiene `first_seen_run` previo: agruparla con las nuevas inflaría el recuento de novedades y ocultaría que algo ya corregido ha vuelto. La distinción permite que la notificación y el neto reflejen reaperturas sin contarlas como descubrimientos.

### sqlx con consultas en tiempo de ejecución

Se usa sqlx con SQLite y consultas resueltas en tiempo de ejecución (`runtime-tokio`, sin macros que verifiquen contra la base en compilación). Así el build no depende de `DATABASE_URL` ni de una base preparada, lo que mantiene la CI y el desarrollo libres de un paso de preparación de esquema.

### Sin red en los tests

GitHub, OSV y Valet se simulan con `wiremock`. Los tests no realizan tráfico de red real: los casos de paginación, `401`, `403` con `Retry-After`, `404` `no analysis found`, los lotes de OSV y las respuestas de Valet se sirven desde mocks. Esto hace la suite determinista, rápida y ejecutable sin credenciales.

### Valet no se modifica

La integración con Valet es un cliente HTTP sobre su API de ingesta ya existente, documentada en `/data/rust/valet/docs/api-notifications.md`. `ghwatch` no cambia el otro proyecto: solo hace `POST {VALET_BASE_URL}/api/notifications` esperando `201`. La notificación es best-effort y nunca condiciona el resultado del escaneo ni la escritura del informe.
