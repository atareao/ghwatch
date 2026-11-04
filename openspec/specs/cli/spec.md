# cli Specification

## Purpose
Superficie de línea de comandos del binario `ghwatch`: subcomandos, resolución de configuración, códigos de salida y su contrato observable.

## Requirements

### Requirement: El binario SHALL exponer los subcomandos `scan`, `report`, `diff`, `history`, `notify` e `init`

El binario `ghwatch` SHALL exponer exactamente los subcomandos `scan`, `report`, `diff`, `history`, `notify` e `init`, y SHALL NOT exigir argumentos para mostrar la ayuda.

#### Scenario: Ayuda del binario

- **Given** el binario compilado
- **When** se ejecuta `ghwatch --help`
- **Then** la salida nombra los seis subcomandos
- **And** el código de salida es 0

#### Scenario: Subcomando desconocido

- **Given** el binario compilado
- **When** se ejecuta `ghwatch volar`
- **Then** se informa del uso correcto
- **And** el código de salida es 2 (error de uso)

### Requirement: La configuración SHALL resolverse con precedencia flags > entorno > fichero > defecto

Los valores de ejecución SHALL resolverse en este orden: **flags de línea de comandos**, luego **variables de entorno**, luego `~/.config/ghwatch/config.toml` y, por último, los **valores por defecto**. La opción `--config <ruta>` SHALL permitir usar otro fichero. El fichero SHALL tener permisos `600`; uno más laxo SHALL rechazarse con un aviso y su contenido SHALL NOT usarse.

#### Scenario: El flag gana al entorno

- **Given** `VALET_BASE_URL` en el entorno y `--valet-base-url` en la línea de comandos
- **When** se resuelve la configuración
- **Then** se usa el valor del flag

#### Scenario: Fichero con permisos laxos

- **Given** un `config.toml` con permisos `644`
- **When** se ejecuta un subcomando que lee la configuración
- **Then** se advierte por el registro
- **And** el fichero no se usa
- **And** ningún token que contuviera aparece en la salida

### Requirement: Ningún token SHALL aparecer en la salida

Ni la salida estándar, ni el registro, ni los informes SHALL contener el token de GitHub ni el token de ingesta de Valet.

#### Scenario: Ejecución verbosa

- **Given** un token de GitHub y uno de ingesta configurados
- **When** se ejecuta `ghwatch scan -vv`
- **Then** ni la salida ni el registro contienen el valor de ninguno de los dos tokens

### Requirement: `ghwatch scan` SHALL escanear, enriquecer, persistir, informar y notificar

`ghwatch scan` SHALL descubrir el inventario, recolectar las alertas de las tres fuentes, enriquecerlas con OSV salvo `--no-enrich`, persistirlas en SQLite, escribir el informe (consola y ficheros) y notificar a Valet **best-effort**. `--include-forks` y `--include-archived` SHALL incorporar al inventario los repositorios que el filtro por defecto excluye. La notificación SHALL ser silenciosa salvo que haya alertas nuevas o resueltas, y `--force` SHALL forzarla.

#### Scenario: Escaneo con novedades

- **Given** un inventario con alertas y una configuración con token de ingesta
- **When** se ejecuta `ghwatch scan`
- **Then** el informe Markdown y el JSON quedan escritos
- **And** Valet recibe una notificación
- **And** el código de salida es 0

#### Scenario: Valet no responde

- **Given** que Valet no responde
- **When** se ejecuta `ghwatch scan`
- **Then** el informe queda escrito y completo
- **And** el código de salida sigue siendo 0
- **And** se registra un aviso

#### Scenario: Inclusión de forks y archivados

- **Given** un inventario con repositorios fork y archivados
- **When** se ejecuta `ghwatch scan --include-forks --include-archived`
- **Then** el inventario incluye esos repositorios

#### Scenario: Segundo escaneo idéntico

- **Given** un escaneo ya persistido
- **When** se repite `ghwatch scan` sin que haya cambiado nada
- **Then** el resumen informa de 0 nuevas y 0 resueltas

### Requirement: Los subcomandos de consulta SHALL leer solo del almacenamiento local

`report`, `diff` y `history` SHALL leer únicamente de almacenamiento local —la base de datos y los ficheros de informe— y SHALL NOT realizar peticiones a GitHub, a OSV ni a Valet. `notify` SHALL poder enviar a Valet el informe ya escrito, que es su propósito, pero nunca vuelve a escanear ni contacta con GitHub ni con OSV.

#### Scenario: Consulta sin credenciales

- **Given** que no hay token de GitHub disponible
- **When** se ejecuta `ghwatch history`
- **Then** la salida muestra la serie almacenada
- **And** el código de salida es 0

### Requirement: `ghwatch diff` SHALL mostrar los cambios desde la última ejecución

`ghwatch diff` SHALL mostrar las alertas nuevas y resueltas de la última ejecución almacenada, leídas de su instantánea histórica.

#### Scenario: Diff tras una ejecución

- **Given** dos ejecuciones almacenadas con cambios entre ellas
- **When** se ejecuta `ghwatch diff`
- **Then** la salida lista las alertas nuevas y las resueltas de la última ejecución

### Requirement: `ghwatch notify` SHALL enviar el último informe sin reescanear

`ghwatch notify` SHALL enviar a Valet el resumen del último escaneo almacenado, SHALL NOT contactar con GitHub y SHALL aceptar `--force` para forzar el envío.

#### Scenario: Envío del último informe

- **Given** un escaneo almacenado y una configuración con token de ingesta
- **When** se ejecuta `ghwatch notify --force`
- **Then** Valet recibe una notificación
- **And** no se realiza ninguna petición a GitHub

### Requirement: `ghwatch init` SHALL preparar la configuración y la base de datos

`ghwatch init` SHALL crear la base de datos aplicando las migraciones y SHALL escribir una plantilla de `~/.config/ghwatch/config.toml` con permisos `600`, sin sobrescribir un fichero existente.

#### Scenario: Inicialización en un directorio limpio

- **Given** que no existe ni la base de datos ni el fichero de configuración
- **When** se ejecuta `ghwatch init`
- **Then** la base de datos existe con su esquema aplicado
- **And** el fichero de configuración existe con permisos `600`

#### Scenario: Configuración preexistente

- **Given** un `config.toml` ya existente
- **When** se ejecuta `ghwatch init`
- **Then** el fichero no se sobrescribe

### Requirement: Los códigos de salida SHALL ser estables y documentados

Los códigos de salida SHALL ser: `0` éxito (incluidos «Valet no responde» y «sin novedades»), `2` error de uso, `3` token de GitHub no resoluble, `4` fallo fatal de la API de GitHub y `5` fallo fatal de la base de datos.

#### Scenario: Sin token de GitHub

- **Given** que no hay token de GitHub disponible
- **When** se ejecuta `ghwatch scan`
- **Then** la salida explica cómo aportar el token
- **And** el código de salida es 3
- **And** no se realiza ninguna petición a GitHub

#### Scenario: Fallo fatal de GitHub

- **Given** que el descubrimiento del inventario falla
- **When** se ejecuta `ghwatch scan`
- **Then** el código de salida es 4

### Requirement: `--quiet` y `-v` SHALL controlar el progreso y el registro

`--quiet` SHALL silenciar la barra de progreso y la salida informativa; `-v` y `-vv` SHALL aumentar el nivel de registro. `--quiet` y `-v` SHALL NOT alterar el resultado del escaneo.

#### Scenario: Modo silencioso

- **Given** un escaneo ejecutado con `--quiet`
- **When** termina
- **Then** no se dibuja barra de progreso
- **And** el código de salida es 0

### Requirement: La base de datos y los informes SHALL tener ubicaciones por defecto configurables

La base de datos SHALL residir por defecto en `~/.local/share/ghwatch/ghwatch.db` y los informes en `~/.local/share/ghwatch/reports`, con nombres que incluyan la fecha. `--db <ruta>` y `--report-dir <ruta>` SHALL permitir otras ubicaciones, creando el directorio si falta.

#### Scenario: Ubicaciones por defecto

- **Given** una ejecución sin `--db` ni `--report-dir`
- **When** termina `ghwatch scan`
- **Then** la base de datos y los informes existen bajo `~/.local/share/ghwatch/`

#### Scenario: Ubicaciones explícitas

- **Given** `--db /tmp/x.db --report-dir /tmp/informes`
- **When** termina `ghwatch scan`
- **Then** la base de datos y los informes están en esas rutas
