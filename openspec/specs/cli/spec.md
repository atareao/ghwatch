# cli Specification

## Purpose
Superficie de línea de comandos del binario `ghwatch`: subcomandos, resolución de configuración, códigos de salida y su contrato observable.

## Requirements

### Requirement: El binario SHALL exponer los subcomandos `scan`, `report`, `diff`, `history`, `repos`, `notify` e `init`

El binario `ghwatch` SHALL exponer exactamente los subcomandos `scan`, `report`, `diff`, `history`, `repos`, `notify` e `init`, y SHALL NOT exigir argumentos para mostrar la ayuda.

#### Scenario: Ayuda del binario

- **Given** el binario compilado
- **When** se ejecuta `ghwatch --help`
- **Then** la salida nombra los siete subcomandos
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

`report`, `diff`, `history` y `repos` SHALL leer únicamente de almacenamiento local —la base de datos y los ficheros de informe— y SHALL NOT realizar peticiones a GitHub, a OSV ni a Valet. `notify` SHALL poder enviar a Valet el informe ya escrito, que es su propósito, pero nunca vuelve a escanear ni contacta con GitHub ni con OSV.

#### Scenario: Consulta sin credenciales

- **Given** que no hay token de GitHub disponible
- **When** se ejecuta `ghwatch history`
- **Then** la salida muestra la serie almacenada
- **And** el código de salida es 0

#### Scenario: Listado de repositorios sin credenciales

- **Given** que no hay token de GitHub disponible y que la base de datos tiene alertas almacenadas
- **When** se ejecuta `ghwatch repos`
- **Then** la salida lista los repositorios con sus alertas
- **And** el código de salida es 0
- **And** no se realiza ninguna petición de red

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

### Requirement: `ghwatch repos` SHALL listar los repositorios con sus alertas abiertas

`ghwatch repos` SHALL listar, para cada repositorio con al menos una alerta **abierta**, su nombre y su número de alertas abiertas, junto con el desglose por severidad. El orden SHALL ser por número de alertas descendente y, a igualdad, por nombre ascendente. La salida SHALL incluir, además, el total de repositorios conocidos, el de repositorios con alertas y el de alertas contadas. Si no hay ninguna alerta abierta, SHALL informarlo y terminar con éxito.

#### Scenario: Listado con varios repositorios

- **Given** una base de datos con alertas abiertas en tres repositorios, con recuentos 5, 5 y 2
- **When** se ejecuta `ghwatch repos`
- **Then** aparecen los tres repositorios
- **And** los dos de recuento 5 aparecen antes que el de recuento 2
- **And** los dos de recuento 5 aparecen ordenados por nombre ascendente
- **And** cada repositorio muestra su desglose por severidad
- **And** el código de salida es 0

#### Scenario: Solo se cuentan las alertas abiertas

- **Given** una base de datos con alertas abiertas y resueltas en el mismo repositorio
- **When** se ejecuta `ghwatch repos`
- **Then** el recuento del repositorio incluye solo las alertas abiertas

#### Scenario: Base de datos sin alertas abiertas

- **Given** una base de datos sin ninguna alerta abierta
- **When** se ejecuta `ghwatch repos`
- **Then** la salida informa de que no hay repositorios con alertas
- **And** el código de salida es 0

#### Scenario: Base de datos inexistente

- **Given** que la base de datos no existe
- **When** se ejecuta `ghwatch repos`
- **Then** la salida informa de que no hay datos
- **And** el código de salida es 0

### Requirement: `ghwatch repos` SHALL acotar la salida con `--limit`/`--all` y filtrarla con `--min-severity`

`--limit <N>` SHALL acotar el número de repositorios mostrados y su valor por defecto SHALL ser 20. `--limit 0` y `--all` SHALL mostrar todos los repositorios, y `--all` SHALL prevalecer si se combinan. `--min-severity <critical|high|medium|low|unknown>` SHALL conservar solo los repositorios con al menos una alerta abierta de esa severidad o superior, sin alterar el recuento ni el desglose que se muestran de cada repositorio. Un valor fuera del conjunto de severidades SHALL ser un error de uso.

#### Scenario: Límite por defecto

- **Given** una base de datos con 30 repositorios con alertas abiertas
- **When** se ejecuta `ghwatch repos` sin opciones de acotación
- **Then** se muestran como máximo 20 repositorios
- **And** el código de salida es 0

#### Scenario: Todos los repositorios

- **Given** una base de datos con 30 repositorios con alertas abiertas
- **When** se ejecuta `ghwatch repos --all`
- **Then** se muestran los 30 repositorios

#### Scenario: Filtro por severidad mínima

- **Given** una base de datos donde solo dos repositorios tienen alertas críticas
- **When** se ejecuta `ghwatch repos --min-severity critical`
- **Then** aparecen exactamente esos dos repositorios
- **And** el recuento de cada uno incluye sus alertas de todas las severidades

#### Scenario: Severidad fuera del conjunto

- **Given** el binario compilado
- **When** se ejecuta `ghwatch repos --min-severity catastrofica`
- **Then** se informa del uso correcto
- **And** el código de salida es 2 (error de uso)

### Requirement: `ghwatch repos` SHALL aceptar `--format` y resolver `--db` y `--config`

`ghwatch repos` SHALL aceptar `--format <console|json|markdown>`, con `console` por defecto. `--db <ruta>` y `--config <ruta>` SHALL resolver la ubicación de la base de datos con la misma precedencia que el resto del CLI. La salida SHALL escribirse en la salida estándar y SHALL NOT crear ni modificar ficheros.

#### Scenario: Salida JSON

- **Given** una base de datos con alertas abiertas
- **When** se ejecuta `ghwatch repos --format json`
- **Then** la salida es un documento JSON válido
- **And** no se crea ni se modifica ningún fichero

#### Scenario: Base de datos explícita

- **Given** una base de datos en una ruta alternativa
- **When** se ejecuta `ghwatch repos --db <ruta>`
- **Then** el listado sale de esa base de datos

#### Scenario: Fallo de la base de datos

- **Given** una base de datos ilegible
- **When** se ejecuta `ghwatch repos`
- **Then** se registra un error
- **And** el código de salida es 5 (fallo de la base de datos)
