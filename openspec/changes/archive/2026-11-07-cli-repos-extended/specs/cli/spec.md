# cli Specification

## RENAMED Requirements

- FROM: `### Requirement: `ghwatch repos` SHALL listar los repositorios con sus alertas abiertas`
- TO: `### Requirement: `ghwatch repos` SHALL listar los repositorios con sus alertas`

- FROM: `### Requirement: `ghwatch repos` SHALL acotar la salida con `--limit`/`--all` y filtrarla con `--min-severity``
- TO: `### Requirement: `ghwatch repos` SHALL acotar la salida con `--limit`/`--all` y filtrarla con `--min-severity`, `--source` y `--state``

- FROM: `### Requirement: `ghwatch repos` SHALL aceptar `--format` y resolver `--db` y `--config``
- TO: `### Requirement: `ghwatch repos` SHALL aceptar `--format`, `--output`, `--db` y `--config``

## MODIFIED Requirements

### Requirement: `ghwatch repos` SHALL listar los repositorios con sus alertas

`ghwatch repos` SHALL listar, para cada repositorio con al menos una alerta en el estado consultado, su nombre y su número de alertas, junto con el desglose por severidad. El orden SHALL ser por número de alertas descendente y, a igualdad, por nombre ascendente. Con `--include-empty` SHALL incluir además los repositorios del inventario sin ninguna alerta, con recuento y desglose a cero, ordenados después de los que tienen alertas. La salida SHALL incluir el total de repositorios conocidos, el de repositorios del listado y el de alertas contadas. Si el listado queda vacío, SHALL informarlo y terminar con éxito.

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
- **And** con `ghwatch repos --state all` el recuento incluye las abiertas y las resueltas

#### Scenario: Base de datos sin alertas abiertas

- **Given** una base de datos sin ninguna alerta abierta
- **When** se ejecuta `ghwatch repos`
- **Then** la salida informa de que no hay repositorios que mostrar
- **And** el código de salida es 0

#### Scenario: Repositorios sin alertas

- **Given** una base de datos con un repositorio sin ninguna alerta
- **When** se ejecuta `ghwatch repos --include-empty`
- **Then** el repositorio aparece con recuento y desglose a cero
- **And** aparece después de los repositorios con alertas
- **And** sin `--include-empty` no aparece

#### Scenario: Base de datos inexistente

- **Given** que la base de datos no existe
- **When** se ejecuta `ghwatch repos`
- **Then** la salida informa de que no hay datos
- **And** el código de salida es 0

### Requirement: `ghwatch repos` SHALL acotar la salida con `--limit`/`--all` y filtrarla con `--min-severity`, `--source` y `--state`

`--limit <N>` SHALL acotar el número de repositorios mostrados y su valor por defecto SHALL ser 20. `--limit 0` y `--all` SHALL mostrar todos los repositorios, y `--all` SHALL prevalecer si se combinan. `--min-severity <critical|high|medium|low|unknown>` SHALL conservar solo los repositorios con al menos una alerta, en el estado consultado, de esa severidad o superior, sin alterar el recuento ni el desglose que se muestran de cada repositorio. `--source <dependabot|secret|code>` SHALL contar solo las alertas de esa fuente. `--state <open|resolved|all>` SHALL elegir el estado de las alertas contadas, con `open` por defecto. Los filtros SHALL ser acumulativos y SHALL aplicarse todos a la vez. Un valor fuera del conjunto de cualquiera de las opciones SHALL ser un error de uso.

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

#### Scenario: Filtro por fuente

- **Given** una base de datos con alertas de Dependabot y de secret scanning en el mismo repositorio
- **When** se ejecuta `ghwatch repos --source dependabot`
- **Then** el recuento del repositorio incluye solo sus alertas de Dependabot

#### Scenario: Estado resuelto

- **Given** una base de datos con alertas abiertas y resueltas en el mismo repositorio
- **When** se ejecuta `ghwatch repos --state resolved`
- **Then** solo aparecen los repositorios con alertas resueltas
- **And** el recuento del repositorio incluye solo sus alertas resueltas

#### Scenario: Fuente fuera del conjunto

- **Given** el binario compilado
- **When** se ejecuta `ghwatch repos --source github`
- **Then** se informa del uso correcto
- **And** el código de salida es 2 (error de uso)

### Requirement: `ghwatch repos` SHALL aceptar `--format`, `--output`, `--db` y `--config`

`ghwatch repos` SHALL aceptar `--format <console|json|markdown>`, con `console` por defecto. `--output <ruta>` SHALL escribir la salida formateada en ese fichero —creándolo o sobrescribiéndolo— y SHALL NOT emitirla por la salida estándar; sin `--output`, la salida SHALL escribirse en la salida estándar. `--db <ruta>` y `--config <ruta>` SHALL resolver la ubicación de la base de datos con la misma precedencia que el resto del CLI. Salvo por `--output`, `repos` SHALL NOT crear ni modificar ficheros.

#### Scenario: Salida JSON

- **Given** una base de datos con alertas abiertas
- **When** se ejecuta `ghwatch repos --format json`
- **Then** la salida es un documento JSON válido
- **And** no se crea ni se modifica ningún fichero

#### Scenario: Salida a fichero

- **Given** una base de datos con alertas abiertas y una ruta de destino escribible
- **When** se ejecuta `ghwatch repos --format markdown --output <ruta>`
- **Then** el fichero contiene el listado en Markdown
- **And** la salida estándar queda vacía
- **And** el código de salida es 0

#### Scenario: Fallo al escribir el fichero

- **Given** una ruta de destino en una carpeta inexistente
- **When** se ejecuta `ghwatch repos --output <ruta>`
- **Then** se registra un error
- **And** el código de salida es 5 (fallo local)

#### Scenario: Base de datos explícita

- **Given** una base de datos en una ruta alternativa
- **When** se ejecuta `ghwatch repos --db <ruta>`
- **Then** el listado sale de esa base de datos

#### Scenario: Fallo de la base de datos

- **Given** una base de datos ilegible
- **When** se ejecuta `ghwatch repos`
- **Then** se registra un error
- **And** el código de salida es 5 (fallo de la base de datos)
