use anyhow::{anyhow, Result};
use pyo3::{prelude::*, types::PyDict};

const SOURCE: &str = include_str!("../lib/decode_demcon3/mineField.py");

enum Mode {
    Beginner,
    Intermediate,
    Expert,
}

#[derive(Debug)]
struct Minefield<'a> {
    field: &'a PyAny,
    width: usize,
    height: usize,
    number_of_mines: usize,
}

impl<'a> Minefield<'a> {
    fn new(py: Python<'a>, mode: Mode) -> Result<Self> {
        let module = PyModule::from_code(py, SOURCE, "mineField", "mineField")?;
        let minefield = module.getattr("MineField")?;

        let preset = match mode {
            Mode::Beginner => "BEGINNER_FIELD",
            Mode::Intermediate => "INTERMEDIATE_FIELD",
            Mode::Expert => "EXPERT_FIELD",
        };

        let kwargs = module
            .getattr(preset)?
            .downcast::<PyDict>()
            .map_err(|e| anyhow!("{}", e))?;

        let width: usize = PyAny::get_item(kwargs, "width")?.extract()?;
        let height: usize = PyAny::get_item(kwargs, "height")?.extract()?;
        let number_of_mines: usize = PyAny::get_item(kwargs, "number_of_mines")?.extract()?;

        let field = minefield.call((), Some(kwargs))?;

        Ok(Self {
            field,
            width,
            height,
            number_of_mines,
        })
    }

    fn sweep_cell(&self, column: usize, row: usize) -> Result<usize> {
        let result = self.field.call_method("sweep_cell", (column, row), None)?;
        Ok(result.extract()?)
    }
}

fn main() -> Result<()> {
    Python::with_gil(|py| {
        let minefield = Minefield::new(py, Mode::Beginner)?;

        println!("{:?}", minefield);

        let result = minefield.sweep_cell(0, 0)?;

        println!("{:?}", result);

        let result = minefield.sweep_cell(1, 1)?;

        println!("{:?}", result);

        Ok(())
    })
}
