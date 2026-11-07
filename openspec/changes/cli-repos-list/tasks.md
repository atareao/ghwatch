# Tasks: cli-repos-list

> **Todas las casillas empiezan sin marcar. No se escribe ni una línea de código ni un solo test antes de que el usuario apruebe este change.** El trabajo empieza por la sección 0, que es un punto de parada obligatorio.

## 0. Aprobación (punto de parada del protocolo)

- [x] 0.1 Presentar este change (proposal + specs + design + tasks) y esperar aprobación explícita antes de tocar código.

**Punto de parada obligatorio.** Ninguna casilla de las secciones 1 a 5 se marca hasta que exista una aprobación explícita del usuario sobre el conjunto proposal + specs + design + tasks. Sin esa aprobación no se añade el subcomando, ni la consulta agregada, ni los formatos, ni los tests.

## 1. Agregación por repositorio en el almacén

- [x] 1.1 RED — escribir en `tests/store.rs` (dentro de `mod store`) el test de la consulta agregada sobre una base en memoria con alertas abiertas y resueltas repartidas en tres repositorios, que debe fallar porque la consulta no existe; comando `cargo test store`; pegar la salida en rojo.
- [x] 1.2 RED — escribir los tests del orden total (recuento descendente y, a igualdad, nombre ascendente), del desglose por severidad y de la exclusión de las alertas resueltas; comando `cargo test store`; pegar la salida en rojo.
- [x] 1.3 RED — escribir el test del filtro por severidad mínima (conserva el repositorio con al menos una alerta de esa severidad o superior y **no** recorta su recuento ni su desglose); comando `cargo test store`; pegar la salida en rojo.
- [x] 1.4 GREEN — implementar en `src/store/query.rs` la consulta agregada (`GROUP BY repo` con `state = 'open'`) y el tipo de resultado con recuento total, desglose por severidad y el número de repositorios conocidos; confirmar con `cargo test store` y `cargo check`.
- [x] 1.5 REFACTOR — documentar el tipo y la consulta con doc-comments y confirmar `cargo clippy --all-targets -- -D warnings` y `cargo fmt --all --check`.
- [x] 1.6 Auditoría con `@rust-reviewer` de la consulta agregada y de su cobertura.

### 1.7 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la sección 1 (veredicto: aprobado con reservas). Aplicados con TDD (rojo → verde).

- [x] 1.7.1 Hallazgo 1 — el test de orden inserta los repositorios empatados en orden inverso al alfabético, de modo que el desempate por nombre no puede confundirse con el orden de inserción. Verificación: `cargo test store`.
- [x] 1.7.2 Hallazgo 2 — el bucket `unknown` pasa a `severity IS NULL OR severity NOT IN ('critical','high','medium','low')`, con un test que inserta una severidad ajena y comprueba que cuenta como desconocida y que el invariante del desglose se mantiene. Verificación: `cargo test store`.
- [x] 1.7.3 Hallazgos 3 y 4 — comentario en el SQL que ata los literales a `Severity::as_str()`/`rank()`, y nota en el doc-comment sobre las dos lecturas no atómicas. Deuda consciente (variante nueva del enum, instantánea no atómica) documentada como follow-up en `docs/cli-repos-list.md`. Verificación: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all --check`.

## 2. Presentación del listado (consola, Markdown y JSON)

- [x] 2.1 RED — escribir en `tests/report.rs` (dentro de `mod report`) los tests de los tres formatos: cabecera de totales y una línea por repositorio en consola; tabla con una columna por severidad en Markdown; documento JSON con `schema_version`, totales y lista de repositorios; comando `cargo test report`; pegar la salida en rojo.
- [x] 2.2 GREEN — implementar el módulo de presentación del listado bajo `src/report/`, sobre el tipo de la sección 1 y con el reloj inyectado para el instante del documento; confirmar con `cargo test report`.
- [x] 2.3 RED — escribir los tests de determinismo (mismo estado y mismo instante → salida idéntica) y del listado vacío (sin repositorios con alertas); comando `cargo test report`; pegar la salida en rojo.
- [x] 2.4 GREEN — cubrir el listado vacío y el determinismo; confirmar con `cargo test report`.
- [x] 2.5 Auditoría con `@rust-reviewer` del módulo de presentación y de su cobertura.

### 2.6 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la sección 2 (veredicto: aprobado con reservas). Aplicados con TDD (rojo → verde).

- [x] 2.6.1 Hallazgo 1 — test de la rama Markdown vacía (`| (ninguno) | 0 | 0 | 0 | 0 | 0 | 0 |`). Verificación: `cargo test report`.
- [x] 2.6.2 Hallazgo 2 — el test de determinismo se renombra y documenta como guarda de pureza (el orden total de las filas se garantiza en `store::repo_alert_counts`). Verificación: `cargo test report`.
- [x] 2.6.3 Hallazgo 3 — se fija por escrito la semántica de los totales: `repos_with_alerts` y `total_alerts` son post-filtro de `--min-severity` y pre-límite de `--limit`; `known_repos` es el total de la base de datos. Verificación: `cargo test report`.
- [x] 2.6.4 Hallazgo 4 — imports de `store` agrupados en una sola línea. Verificación: `cargo fmt --all --check`.

## 3. Subcomando `repos`

- [x] 3.1 RED — escribir en `tests/cli.rs` (dentro de `mod cli`) el test de `ghwatch repos` sobre una base en memoria: listado ordenado, totales en la cabecera y código de salida 0; comando `cargo test cli`; pegar la salida en rojo.
- [x] 3.2 RED — escribir los tests de `--limit` (defecto 20 y `0` = todos), `--all`, `--min-severity` válido e inválido (código 2), base de datos inexistente (código 0) y `--format json`; comando `cargo test cli`; pegar la salida en rojo.
- [x] 3.3 GREEN — añadir la variante `Repos` al enum `Command`, sus argumentos (`--limit`, `--all`, `--min-severity`, `--format`, `--db`, `--config`) y `ejecutar_repos` siguiendo el patrón de `ejecutar_history`; confirmar con `cargo test cli`.
- [x] 3.4 GREEN — confirmar la ayuda: `ghwatch --help` nombra los siete subcomandos y `ghwatch repos --help` documenta todas sus opciones; comando `cargo test cli`.
- [x] 3.5 REFACTOR — confirmar la puerta de calidad completa: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test` (los 289 tests previos siguen verdes).
- [x] 3.6 Auditoría con `@rust-reviewer` del subcomando, de los códigos de salida y de la precedencia de configuración.

### 3.7 Endurecimiento tras la auditoría

> Cambios derivados de la auditoría de la sección 3 (veredicto: aprobado con reservas).

- [x] 3.7.1 Hallazgo 1 — el aviso de base inexistente pasa a `sin datos locales (ejecuta \`ghwatch scan\`)`. Verificación: `cargo test cli`.
- [x] 3.7.2 Hallazgo 2 — test de CLI de `repos --format markdown`. Verificación: `cargo test cli`.
- [x] 3.7.3 Hallazgo 3 — test de la ayuda de `repos` (todas sus opciones). Verificación: `cargo test cli`.

## 4. Documentación

- [ ] 4.1 Actualizar `AGENTS.md` (los siete subcomandos y el módulo nuevo) y verificar que la lista coincide con la salida de `ghwatch --help`.
- [ ] 4.2 Actualizar `PLAN.md` con el subcomando `repos` y verificar que no contradice la sección de modelo de datos.
- [ ] 4.3 Actualizar `README.md` y `README.es.md` con el subcomando `repos` y un ejemplo de salida; verificar que el ejemplo, ejecutado contra la base real, produce esa forma.
- [ ] 4.4 Crear `docs/cli-repos-list.md` (qué es, ficheros implicados, cómo funciona, variables y configuración, cómo usarlo, decisiones y follow-ups) y verificar que todos los comandos que cita funcionan tal cual.

## 5. Verificación final

- [ ] 5.1 Ejecutar `ghwatch repos --all` contra la base de datos real y comprobar que el total de repositorios con alertas y el de alertas coinciden con `ghwatch report`; pegar la salida.
- [ ] 5.2 Ejecutar la puerta de calidad completa (`cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`) y pegar el resumen.
- [ ] 5.3 Verificar requirement por requirement del change contra la implementación y marcar las casillas de las secciones 1 a 4.
