use anyhow::{anyhow, Result};
use pyo3::{prelude::*, types::PyDict};
use owo_colors::OwoColorize;
use clap::{Parser, Subcommand};

const SOURCE: &str = include_str!("../lib/decode_demcon3/mineField.py");

#[derive(Subcommand)]
enum Mode {
    Beginner,
    Intermediate,
    Expert,
}

#[derive(Debug)]
struct Minefield<'a> {
    field: &'a PyAny,
    width: i32,
    height: i32,
    number_of_mines: i32,
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

        let width: i32 = PyAny::get_item(kwargs, "width")?.extract()?;
        let height: i32 = PyAny::get_item(kwargs, "height")?.extract()?;
        let number_of_mines: i32 = PyAny::get_item(kwargs, "number_of_mines")?.extract()?;

        let field = minefield.call((), Some(kwargs))?;

        Ok(Self {
            field,
            width,
            height,
            number_of_mines,
        })
    }

    fn sweep_cell(&self, column: i32, row: i32) -> Result<u8> {
        let result = self.field.call_method("sweep_cell", (column, row), None)?;
        Ok(result.extract()?)
    }
}

#[derive(Clone, Copy)]
enum Cell {
    Unknown,
    Flag,
    Number(u8),
}

struct Solver<'a> {
    minefield: &'a Minefield<'a>,
    board: Vec<Cell>,
}

impl<'a> Solver<'a> {
    fn new(minefield: &'a Minefield) -> Result<Self> {
        let size: usize = (minefield.width * minefield.height).try_into()?;
        Ok(Self {
            minefield,
            board: vec![Cell::Unknown; size],
        })
    }

    fn index(&self, col: i32, row: i32) -> Option<usize> {
        if col < 0 || col >= self.minefield.width || row < 0 || row >= self.minefield.height {
            return None;
        }

        let index: usize = (col + row * self.minefield.width).try_into().unwrap();
        Some(index)
    }

    fn get(&self, col: i32, row: i32) -> Option<Cell> {
        self.index(col, row).map(|i| self.board[i])
    }

    fn uncover(&mut self, col: i32, row: i32) -> Result<Cell> {
        let n = self.minefield.sweep_cell(col, row)?;
        let cell = Cell::Number(n);
        let i = self.index(col, row).ok_or(anyhow!("Bad index"))?;
        self.board[i] = cell;
        Ok(cell)
    }

    fn plant_flag(&mut self, col: i32, row: i32) -> Result<()> {
        let i = self.index(col, row).ok_or(anyhow!("Bad index"))?;
        self.board[i] = Cell::Flag;
        Ok(())
    }

    fn neighbors(&self, col: i32, row: i32) -> Vec<(i32, i32, Cell)> {
        let list = [
            (1, 1),
            (1, 0),
            (1, -1),
            (0, 1),
            (0, -1),
            (-1, 1),
            (-1, 0),
            (-1, -1),
        ];
        let r: Vec<(i32, i32, Cell)> = list
            .iter()
            .map(|(c, r)| {
                self.get(col + c, row + r)
                    .map(|cell| (col + c, row + r, cell))
            })
            .filter_map(|id| id)
            .collect();

        r
    }

    fn solve(&mut self) -> Result<()> {
        let mut active: Vec<(i32, i32)> = Vec::new();
        let mut new: Vec<(i32, i32)> = Vec::new();
        let mut old: Vec<(i32, i32)> = Vec::new();

        // First guess
        new.push((0, 0));

        loop {

            active.clear();
            active.append(&mut new);
            active.append(&mut old);


            for (col, row) in active.iter().copied() {
                let mut cell = self.get(col, row).ok_or(anyhow!("Bad active cell location"))?;
                if matches!(cell, Cell::Unknown) {
                    cell = self.uncover(col, row)?;
                }

                match cell {
                    Cell::Number(bombs) => {
                        let bombs: i32 = bombs.into();
                        let neighbors = self.neighbors(col, row);
                        let flags: i32 = neighbors.iter().filter(|(_,_,cell)| matches!(cell, Cell::Flag)).count().try_into().unwrap();
                        let unknowns: i32 = neighbors.iter().filter(|(_,_,cell)| matches!(cell, Cell::Unknown)).count().try_into().unwrap();

                        if bombs == flags {
                            let iter = neighbors.iter().filter_map(|(col, row, cell)| matches!(cell, Cell::Unknown).then(|| (*col, *row)));
                            new.extend( iter );
                        } else if unknowns + flags == bombs {

                            let mark: Vec<(i32, i32)> = neighbors.iter().filter_map(|(col, row, cell)| matches!(cell, Cell::Unknown).then(|| (*col, *row))).collect();

                            for (col, row) in mark.iter().copied() {
                                self.plant_flag(col,row)?;
                            }

                            new.extend( mark );
                        } else {
                            old.push((col, row));
                        }

                    }
                    _ => (),
                }
            }

            if new.is_empty() {
                break;
            }
        }

        Ok(())
    }

    fn solved(&self) -> bool {
        let flags: i32 = self.board.iter().filter(|cell| matches!(cell, Cell::Flag)).count().try_into().unwrap();
        let unknowns: i32 = self.board.iter().filter(|cell| matches!(cell, Cell::Unknown)).count().try_into().unwrap();
        unknowns == 0 && flags == self.minefield.number_of_mines
    }

    fn show(&self) {
        for row in 0..self.minefield.height {
            for col in 0..self.minefield.width {
                match self.get(col, row) {
                    Some(Cell::Flag) => print!("{} ", "F".bold().red()),
                    Some(Cell::Unknown) => print!(". "),
                    Some(Cell::Number(0)) => print!("  "),
                    Some(Cell::Number(x)) => print!("{} ", x),
                    _ => (),
                }
            }
            println!("");
        }
    }
}


#[derive(Parser)]
#[clap(about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    mode: Mode,
}

fn main() -> Result<()> {

    let cli = Cli::parse();

    Python::with_gil(|py| {
        let minefield = Minefield::new(py, cli.mode)?;

        let mut solver = Solver::new(&minefield)?;

        solver.solve()?;

        solver.show();

        println!("");
        println!("Solved: {}", solver.solved());

        Ok(())
    })
}
