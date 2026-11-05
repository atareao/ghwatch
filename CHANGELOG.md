# Changelog
## [0.1.0] - 2026-11-05

### Bug Fixes

- *(github)* Clasificar 3xx como Http (S3) y analizar la consulta de per_page por pares (S9)
- *(github)* Distinguir 403 de capacidad deshabilitada con GithubError::Disabled (S1)
- *(github)* Acotar la paginación con max_pages y detección de URL repetida (S2)
- *(github)* Exponer los fallos de las tareas en for_each_concurrent (S4)

### Documentation

- *(openspec)* Marcar como cumplidas 0.1 y 1.1-1.7
- *(auth)* Aclarar que el token solo se expone por expose_secret y documentar la vía asíncrona
- *(openspec)* Marcar 1.8 y añadir 1.9 con el endurecimiento de la auditoría
- *(openspec)* Marcar 2.1-2.7 del cliente GitHub como cumplidas
- *(github)* Documentar que la espera conservadora exige leer Remaining (S6)
- *(openspec)* Marcar 2.8 y añadir 2.9 con el endurecimiento de la auditoría
- Añadir README, licencia MIT, CI y metadatos del paquete
- Añadir AGENTS.md con la guía operativa para agentes
- *(agents)* Actualizar el estado del proyecto (fases 3 y 4)
- *(agents)* Actualizar el estado del proyecto (fase 5)
- *(agents)* Actualizar el estado del proyecto (fase 6)
- *(spec)* Delta de la capacidad `cli` y tareas de la fase 7
- *(spec)* Precisar el alcance local de las consultas y cerrar las tareas 7.A
- *(tasks)* Cerrar las tareas de operación (7.11-7.14)
- Actualizar el estado, el recuento de tests y la deriva del flag de Valet
- *(spec)* Cerrar las tareas 7.15-7.17 del change ghwatch-scanner
- *(openspec)* Archivar el change ghwatch-scanner y publicar sus specs

### Features

- *(auth)* Resolver la credencial de GitHub sin exponer el token
- *(github)* Cliente HTTP con paginación, cabeceras, control de límites y reintentos
- *(scan)* Inventario, recolección y orquestación (fase 3)
- *(store)* Persistencia, ciclo de vida y deriva (fase 4)
- *(osv)* Enriquecimiento OSV por lotes y priorización (fase 5)
- *(report,notify)* Informes y notificación a Valet (fase 6)
- *(cli)* CLI real con subcomandos, configuración y códigos de salida (fase 7.A)

### Miscellaneous Tasks

- Bootstrap ghwatch con plan y especificación OpenSpec
- *(scaffold)* Inicializar crate ghwatch con dependencias e higiene
- *(merge)* Integrar fase 1 (andamiaje y modulo auth)
- *(merge)* Integrar fase 2 (cliente GitHub auditado y endurecido)
- *(merge)* Integrar la documentación pública (README, licencia y CI)
- *(ops)* Systemd, justfile, README y CI de la fase 7.B
- Apuntar las referencias al change a las specs archivadas
- *(release)* Construir y publicar la GitHub Release desde el tag

### Refactor

- *(auth)* Interpretar la salida de gh en una función pura y estricta en UTF-8
- *(auth)* Validar Token con Token::new y eliminar los From públicos
- *(auth)* Añadir la vía asíncrona con tokio::process

### Styling

- *(github)* Aplicar rustfmt a los tests nuevos

### Testing

- *(auth)* Rojo del escenario con GITHUB_TOKEN presente
- *(auth)* Rojo del escenario sin GITHUB_TOKEN y con gh disponible
- *(auth)* Rojo del escenario sin credenciales y de la redacción del token
- *(auth)* Cubrir la salida vacía real, el fallthrough a gh y el recorte
- *(github)* Rojo de la fase 2 con el módulo de cliente ausente
- *(github)* Rojo de S3 (3xx como Http), S9 (per_page falso positivo) y B1 (paginación por cursor)
- *(github)* Acotar la concurrencia observada y documentar el semáforo (S5)
