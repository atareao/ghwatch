# reporting Specification

## Purpose
Presentación del estado actual y de la evolución en consola, Markdown y JSON.

## Requirements

### Requirement: La salida de consola SHALL agrupar por severidad y ordenar por prioridad

La salida de consola SHALL presentar una tabla con color por severidad, agrupada por severidad y, dentro de cada grupo, ordenada por prioridad descendente.

#### Scenario: Resumen con varias severidades

- **Given** alertas de severidad `critical`, `high` y `medium`
- **When** se imprime el resumen
- **Then** las alertas aparecen agrupadas por severidad
- **And** dentro de cada grupo aparecen ordenadas por prioridad descendente

### Requirement: El informe Markdown SHALL contener la situación actual

El informe Markdown SHALL contener los totales por severidad, el desglose por ecosistema y una tabla con los 15 repositorios con más alertas.

#### Scenario: Informe con los datos del escaneo

- **Given** los datos verificados (≈1.333 alertas en 90 repositorios de 367 escaneados)
- **When** se genera el informe
- **Then** el informe muestra el total, el desglose por severidad y el desglose por ecosistema
- **And** incluye exactamente 15 repositorios en la tabla de cabecera

### Requirement: El informe Markdown SHALL contener la evolución

El informe Markdown SHALL incluir las últimas N ejecuciones con fecha, nuevas, resueltas y neto, acompañadas de una representación visual de la tendencia.

#### Scenario: Serie de cinco ejecuciones

- **Given** cinco ejecuciones almacenadas
- **When** se genera el informe
- **Then** aparece una fila por ejecución con nuevas, resueltas y neto
- **And** la tendencia se representa de forma gráfica

### Requirement: El informe Markdown SHALL proponer acciones

El informe Markdown SHALL incluir una lista de paquetes con la versión que corrige, ordenada por número de repositorios afectados.

#### Scenario: Alertas que comparten paquete y corrección

- **Given** varias alertas que comparten paquete y versión corregida
- **When** se genera el informe
- **Then** aparece una entrada por paquete con la versión corregida
- **And** cada entrada indica el número de repositorios afectados

### Requirement: La salida JSON SHALL ser estable y versionada

La salida JSON SHALL declarar un campo `schema_version` y SHALL usar claves estables. El documento completo SHALL ser la única fuente de verdad observable, sin campos dependientes de la ejecución fuera de los documentados.

#### Scenario: Inspección del documento JSON

- **Given** un informe JSON
- **When** se inspecciona
- **Then** contiene `schema_version`
- **And** el documento completo es la única fuente de verdad observable, sin campos dependientes de la ejecución fuera de los documentados

### Requirement: El informe SHALL escribirse en disco con independencia del resultado de la notificación

La escritura del informe SHALL NOT depender del resultado de la notificación. Un fallo de la notificación SHALL NOT dejar el informe sin escribir.

#### Scenario: Valet no responde

- **Given** que Valet no responde
- **When** termina el escaneo
- **Then** el informe Markdown y el JSON están escritos y completos
