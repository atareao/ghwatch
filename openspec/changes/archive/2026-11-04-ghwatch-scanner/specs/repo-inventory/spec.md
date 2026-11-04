# repo-inventory Specification

## Purpose

Descubrimiento del inventario de repositorios sobre el que se ejecuta el escaneo.

## ADDED Requirements

### Requirement: El descubrimiento SHALL enumerar el inventario completo sin truncar

El descubrimiento SHALL enumerar el inventario mediante paginación con `per_page=100`, siguiendo el encabezado `Link` con `rel="next"` hasta agotar las páginas. El cliente SHALL NOT truncar la lista al tamaño de página ni asumir que la primera respuesta contiene el inventario completo.

#### Scenario: Cuenta con inventario repartido en cinco páginas

- **Given** una cuenta con 433 repositorios repartidos en 5 páginas de 100 elementos o menos
- **When** se ejecuta el descubrimiento
- **Then** el inventario contiene exactamente 433 entradas
- **And** se han solicitado las 5 páginas

#### Scenario: Última página sin enlace de continuación

- **Given** una última página que devuelve menos de 100 elementos y no incluye `rel="next"`
- **When** se pagina el inventario
- **Then** la enumeración termina
- **And** no se solicita ninguna página adicional

### Requirement: La selección por defecto SHALL excluir forks y archivados

El descubrimiento SHALL incluir por defecto únicamente repositorios no-fork y no-archivados. Dos flags SHALL reincorporar las categorías excluidas: `--include-forks` y `--include-archived`.

#### Scenario: Selección por defecto frente a inclusión de forks

- **Given** 433 repositorios accesibles de los que 66 son forks o archivados
- **When** se ejecuta el descubrimiento sin flags
- **Then** el inventario contiene 367 repositorios
- **When** se ejecuta con `--include-forks`
- **Then** los forks excluidos se incorporan al inventario

#### Scenario: Inclusión de archivados

- **Given** un repositorio archivado
- **When** se ejecuta el descubrimiento sin flags
- **Then** el repositorio archivado no aparece en el inventario
- **When** se ejecuta con `--include-archived`
- **Then** el repositorio archivado aparece en el inventario

### Requirement: Cada repositorio SHALL registrarse con su metadato de identidad y su estado de alertas

Cada repositorio SHALL registrarse con los campos `name_with_owner`, `private`, `archived`, `fork`, `language`, `pushed_at` y el estado de Dependabot, que SHALL ser uno de `enabled`, `disabled` o `unknown`.

#### Scenario: Registro de metadatos tras el descubrimiento

- **Given** un descubrimiento completado
- **When** se inspecciona la tabla `repos`
- **Then** cada fila tiene `name_with_owner` no vacío y `pushed_at` no nulo

#### Scenario: Repositorio privado

- **Given** un repositorio privado en el inventario
- **When** se persiste su metadato
- **Then** su campo `private` queda a `true`
