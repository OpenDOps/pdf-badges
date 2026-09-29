use lopdf::Document;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use crate::modules::data_structures::*;
use crate::modules::pdf_analyzer;

/// Opens a PDF, asks the page, text, and image modules for a `PdfAnalysis`, and writes it out.
pub struct StructuredPdfAnalyzer;

//TODO: Read q, Q, gs for saving state and restoring state
//TODO: re, W*, n for drawing rectangles and clipping
//TODO: gs, RG, rg for setting color and transparency
//TODO: BT, ET, Tf, Tm, Tj, TJ, Td, TD for text
//TODO: Do is drawing (saving struct object rectangle with it's matrix)
//Get all the objects using this operations
//Merge text elements with same position and get interline spacing, font size, font name, and alignment
impl StructuredPdfAnalyzer {
    /// Analyze a PDF document and return structured data
    pub fn analyze_pdf(pdf_path: &PathBuf) -> Result<PdfAnalysis, Box<dyn std::error::Error>> {
        let file = File::open(pdf_path)?;
        let mut reader = BufReader::new(file);
        let doc = Document::load_from(&mut reader)?;

        let metadata = pdf_analyzer::extract_metadata(&doc)?;
        let pages = pdf_analyzer::extract_pages(&doc)?;

        Ok(PdfAnalysis { pages, metadata })
    }

    /// Save analysis results to JSON file
    pub fn save_to_json(
        analysis: &PdfAnalysis,
        output_path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(analysis)?;
        std::fs::write(output_path, json)?;
        Ok(())
    }

    /// Save analysis results to YAML file
    pub fn save_to_yaml(
        analysis: &PdfAnalysis,
        output_path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let yaml = serde_yaml::to_string(analysis)?;
        std::fs::write(output_path, yaml)?;
        Ok(())
    }
}
