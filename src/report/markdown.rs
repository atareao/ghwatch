//! Informe en Markdown.
//!
//! El informe Markdown reúne la **situación actual** (totales, desglose por
//! severidad y ecosistema, y los repositorios con más alertas), la **evolución**
//! (una fila por ejecución con nuevas, resueltas y neto, más una tendencia
//! gráfica) y las **acciones** propuestas por paquete.

use crate::report::document::ReportDocument;

/// Caracteres de la tendencia gráfica, de menos a más.
const BLOQUES: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

impl ReportDocument {
    /// Renderiza el informe completo en Markdown.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut md = String::with_capacity(4096);
        self.cabecera(&mut md);
        self.situacion_actual(&mut md);
        self.evolucion(&mut md);
        self.acciones(&mut md);
        md
    }

    /// Cabecera con los totales del informe.
    fn cabecera(&self, md: &mut String) {
        md.push_str("# Informe ghwatch\n\n");
        md.push_str(&format!(
            "- Generado: {}\n",
            self.generated_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        md.push_str(&format!("- Ejecución: #{}\n", self.run_id));
        md.push_str(&format!(
            "- Repositorios escaneados: {} (fallidos: {})\n",
            self.scanned, self.failed
        ));
        md.push_str(&format!("- Alertas totales: {}\n", self.total));
        md.push_str(&format!(
            "- Nuevas: {} · Resueltas: {}\n\n",
            self.new, self.resolved
        ));
    }

    /// Situación actual: severidad, ecosistema y peores repositorios.
    fn situacion_actual(&self, md: &mut String) {
        md.push_str("## Situación actual\n\n");
        md.push_str(&format!("Críticas: {}\n", self.by_severity.critical));
        md.push_str(&format!("Altas: {}\n", self.by_severity.high));
        md.push_str(&format!("Medias: {}\n", self.by_severity.medium));
        md.push_str(&format!("Bajas: {}\n", self.by_severity.low));
        md.push_str(&format!("Desconocidas: {}\n\n", self.by_severity.unknown));

        md.push_str("### Ecosistemas\n\n");
        md.push_str("| Ecosistema | Alertas |\n| --- | --- |\n");
        if self.by_ecosystem.is_empty() {
            md.push_str("| (ninguno) | 0 |\n");
        } else {
            for eco in &self.by_ecosystem {
                md.push_str(&format!("| {} | {} |\n", eco.ecosystem, eco.alerts));
            }
        }
        md.push('\n');

        md.push_str("### Repositorios con más alertas\n\n");
        md.push_str("| Repositorio | Alertas |\n| --- | --- |\n");
        if self.worst_repos.is_empty() {
            md.push_str("| (ninguno) | 0 |\n");
        } else {
            for repo in &self.worst_repos {
                md.push_str(&format!("| {} | {} |\n", repo.repo, repo.alerts));
            }
        }
        md.push('\n');
    }

    /// Evolución: una fila por ejecución y la tendencia gráfica.
    fn evolucion(&self, md: &mut String) {
        md.push_str("## Evolución\n\n");
        md.push_str("| Fecha | Nuevas | Resueltas | Neto |\n| --- | --- | --- | --- |\n");
        if self.history.is_empty() {
            md.push_str("| (sin histórico) | 0 | 0 | 0 |\n");
        } else {
            for punto in &self.history {
                md.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    punto.started_at.format("%Y-%m-%d"),
                    punto.new,
                    punto.resolved,
                    punto.net
                ));
            }
        }
        md.push('\n');
        md.push_str(&format!("Tendencia: {}\n\n", self.tendencia()));
    }

    /// Acciones propuestas por paquete.
    fn acciones(&self, md: &mut String) {
        md.push_str("## Acciones propuestas\n\n");
        md.push_str("| Paquete | Versión que corrige | Repos afectados |\n| --- | --- | --- |\n");
        if self.actions.is_empty() {
            md.push_str("| (ninguna) | — | 0 |\n");
        } else {
            for accion in &self.actions {
                md.push_str(&format!(
                    "| {} | {} | {} |\n",
                    accion.package,
                    accion.fixed_version.as_deref().unwrap_or("—"),
                    accion.repos
                ));
            }
        }
    }

    /// Tendencia gráfica del total de alertas, un bloque por ejecución.
    fn tendencia(&self) -> String {
        let totales: Vec<i64> = self.history.iter().map(|punto| punto.total).collect();
        let Some(minimo) = totales.iter().copied().min() else {
            return String::new();
        };
        let maximo = totales.iter().copied().max().unwrap_or(minimo);
        let rango = (maximo - minimo).max(1) as f64;

        totales
            .iter()
            .map(|total| {
                let nivel = (((total - minimo) as f64 / rango) * 7.0).round() as usize;
                BLOQUES[nivel.min(BLOQUES.len() - 1)]
            })
            .collect()
    }
}
