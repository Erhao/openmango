use crate::components::file_picker::FileFilter;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileExportFormat {
    JsonArray,
    JsonLines,
    Csv,
    Excel,
}

impl FileExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::JsonArray => "JSON",
            Self::JsonLines => "JSONL",
            Self::Csv => "CSV",
            Self::Excel => "Excel",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::JsonArray => "json",
            Self::JsonLines => "jsonl",
            Self::Csv => "csv",
            Self::Excel => "xlsx",
        }
    }

    pub fn all() -> &'static [FileExportFormat] {
        &[Self::JsonArray, Self::JsonLines, Self::Csv, Self::Excel]
    }

    pub fn file_filters(self) -> Vec<FileFilter> {
        match self {
            Self::JsonArray => vec![FileFilter::json_array(), FileFilter::all()],
            Self::JsonLines => vec![FileFilter::json_lines(), FileFilter::all()],
            Self::Csv => vec![FileFilter::csv(), FileFilter::all()],
            Self::Excel => vec![FileFilter::excel(), FileFilter::all()],
        }
    }
}
