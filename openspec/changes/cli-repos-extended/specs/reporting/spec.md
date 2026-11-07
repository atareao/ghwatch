# reporting Specification

## MODIFIED Requirements

### Requirement: El listado de repositorios SHALL presentarse en consola, Markdown y JSON con la misma información

El listado de repositorios SHALL presentarse en los tres formatos —consola, Markdown y JSON— con la misma información: los totales (repositorios conocidos, repositorios del listado, repositorios con alertas y alertas contadas), el identificador de la ejecución, los filtros aplicados (estado, fuente, severidad mínima e inclusión de repositorios sin alertas) y, por cada repositorio, su nombre, su número de alertas y su desglose por severidad. El documento JSON SHALL declarar un campo `schema_version` y SHALL usar claves estables. La presentación SHALL ser determinista: el mismo estado de la base de datos y los mismos filtros SHALL producir exactamente la misma salida. Ningún token SHALL aparecer en la salida.

#### Scenario: Salida de consola del listado

- **Given** un listado con varios repositorios
- **When** se presenta en consola
- **Then** aparece una línea de cabecera con los totales
- **And** aparece una línea por repositorio con su nombre, su número de alertas y su desglose por severidad

#### Scenario: Cabecera según el estado consultado

- **Given** un listado de alertas abiertas y otro de alertas resueltas
- **When** se presentan en consola
- **Then** la cabecera del primero dice «alertas abiertas»
- **And** la cabecera del segundo dice «alertas resueltas»

#### Scenario: Tabla Markdown del listado

- **Given** un listado con varios repositorios
- **When** se presenta en Markdown
- **Then** aparece una tabla con una columna por severidad
- **And** aparece una fila por repositorio con su recuento total y su desglose

#### Scenario: Documento JSON del listado

- **Given** un listado con varios repositorios
- **When** se presenta en JSON
- **Then** el documento contiene `schema_version`, los filtros aplicados, los totales y la lista de repositorios
- **And** cada repositorio declara su nombre, su número de alertas y su desglose por severidad

#### Scenario: Determinismo del listado

- **Given** el mismo estado de la base de datos, los mismos filtros y el mismo instante inyectado
- **When** se presenta el listado dos veces
- **Then** las dos salidas son idénticas
