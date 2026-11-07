# cli Specification

## RENAMED Requirements

- FROM: `### Requirement: El binario SHALL exponer los subcomandos `scan`, `report`, `diff`, `history`, `notify` e `init``
- TO: `### Requirement: El binario SHALL exponer los subcomandos `scan`, `report`, `diff`, `history`, `repos`, `notify` e `init``

## MODIFIED Requirements

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

## ADDED Requirements

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
