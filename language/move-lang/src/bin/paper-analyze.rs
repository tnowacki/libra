// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

use move_lang::{
    command_line::{self as cli},
    compiled_unit::{self, CompiledUnit},
};
use move_vm::file_format::*;
use structopt::*;

#[derive(Debug, StructOpt)]
#[structopt(name = "Anayalze", about = "Print stuff for paper")]
pub struct Options {
    /// The source files to check
    #[structopt(name = "PATH_TO_SOURCE_FILE")]
    pub source_files: Vec<String>,

    /// The library files needed as dependencies
    #[structopt(
        name = "PATH_TO_DEPENDENCY_FILE",
        short = cli::DEPENDENCY_SHORT,
        long = cli::DEPENDENCY,
    )]
    pub dependencies: Vec<String>,
}

pub fn main() -> anyhow::Result<()> {
    let Options {
        source_files,
        dependencies,
    } = Options::from_args();

    let (_files, compiled_units) =
        move_lang::move_compile(&source_files, &dependencies, None, None)?;

    let now = std::time::Instant::now();
    let (compiled_units, errors) = compiled_unit::verify_units(compiled_units);
    println!(
        "Miliseconds to verify compiled units: {}",
        now.elapsed().as_millis()
    );
    assert!(errors.is_empty());

    let mut counts = Counts::default();
    for unit in &compiled_units {
        match unit {
            CompiledUnit::Script { script, .. } => count_script(&mut counts, script),
            CompiledUnit::Module { module, .. } => count_module(&mut counts, module),
        }
    }
    counts.print();
    Ok(())
}

#[derive(Default)]
struct Counts {
    imm_borrow_loc: usize,
    mut_borrow_loc: usize,
    imm_borrow_field: usize,
    mut_borrow_field: usize,
    imm_borrow_global: usize,
    mut_borrow_global: usize,
    freeze: usize,
    total_instructions: usize,

    reference_parameters: usize,
    reference_return_values: usize,
    acquires_annotations: usize,

    total_functions: usize,
    functions_with_reference_operations: usize,
    functions_with_reference_signatures: usize,
    functions_with_acquires: usize,

    total_modules: usize,
    modules_with_acquires: usize,
}

impl Counts {
    fn print(self) {
        macro_rules! percent {
            ($x:expr, $y:expr) => {{
                let x = $x;
                let y = $y;
                format!("{}/{} ({:.2}%)", x, y, (x as f64) / (y as f64) * 100.)
            }};
        }

        let total_reference_operations = self.total_reference_operations();
        let Counts {
            imm_borrow_loc,
            mut_borrow_loc,
            imm_borrow_field,
            mut_borrow_field,
            imm_borrow_global,
            mut_borrow_global,
            freeze,
            total_instructions,
            reference_parameters,
            reference_return_values,
            acquires_annotations,
            total_functions,
            functions_with_reference_operations,
            functions_with_reference_signatures,
            functions_with_acquires,
            total_modules,
            modules_with_acquires,
        } = self;
        println!(
            "Total reference operations (not including move/copy/pop): {}",
            total_reference_operations
        );
        println!("  Total borrow local: {}", imm_borrow_loc + mut_borrow_loc);
        println!("    Imm borrow local: {}", imm_borrow_loc);
        println!("    Mut borrow local: {}", mut_borrow_loc);
        println!(
            "  Total borrow field: {}",
            imm_borrow_field + mut_borrow_field
        );
        println!("    Imm borrow field: {}", imm_borrow_field);
        println!("    Mut borrow field: {}", mut_borrow_field);
        println!(
            "  Total borrow global: {}",
            imm_borrow_global + mut_borrow_global
        );
        println!("    Imm borrow global: {}", imm_borrow_global);
        println!("    Mut borrow global: {}", mut_borrow_global);
        println!("  Freeze: {}", freeze);
        println!(
            "Fraction of instructions that are reference instructions: {}",
            percent!(total_reference_operations, total_instructions)
        );
        println!();

        let total_annots = reference_parameters + reference_return_values + acquires_annotations;
        println!("Total reference related annotations: {}", total_annots);
        println!(
            "  Total reference function type annotations: {}",
            reference_parameters + reference_return_values
        );
        println!("    Reference parameters: {}", reference_parameters);
        println!("    Reference return values: {}", reference_return_values);
        println!("  Acquire annotations: {}", acquires_annotations);
        println!();

        println!(
            "Functions with reference operations: {}",
            percent!(functions_with_reference_operations, total_functions)
        );
        println!(
            "Functions with reference signatures: {}",
            percent!(functions_with_reference_signatures, total_functions)
        );
        println!(
            "Functions with acquires: {}",
            percent!(functions_with_acquires, total_functions)
        );
        println!(
            "Modules with acquires: {}",
            percent!(modules_with_acquires, total_modules)
        );
    }

    fn total_reference_operations(&self) -> usize {
        self.imm_borrow_loc
            + self.mut_borrow_loc
            + self.imm_borrow_field
            + self.mut_borrow_field
            + self.imm_borrow_global
            + self.mut_borrow_global
            + self.freeze
    }
}

fn count_module(counts: &mut Counts, module: &CompiledModule) {
    counts.total_modules += 1;
    let before_acquires = counts.acquires_annotations;
    let module = module.as_inner();
    for fdef in &module.function_defs {
        let fhandle = &module.function_handles[fdef.function.0 as usize];
        count_function_signature(
            counts,
            &module.signatures[fhandle.parameters.0 as usize].0,
            &module.signatures[fhandle.return_.0 as usize].0,
            &fdef.acquires_global_resources,
        );
        if let Some(code) = &fdef.code {
            count_instructions(counts, &code.code)
        }
    }
    let after_acquires = counts.acquires_annotations;
    if after_acquires > before_acquires {
        counts.modules_with_acquires += 1;
    }
}

fn count_script(counts: &mut Counts, script: &CompiledScript) {
    let script = script.as_inner();
    count_function_signature(
        counts,
        &script.signatures[script.parameters.0 as usize].0,
        &vec![],
        &vec![],
    );
    count_instructions(counts, &script.code.code)
}

fn count_function_signature(
    counts: &mut Counts,
    parameters: &[SignatureToken],
    return_types: &[SignatureToken],
    acquires: &[StructDefinitionIndex],
) {
    counts.total_functions += 1;
    let mut has_reference = false;
    for parameter in parameters {
        match parameter {
            SignatureToken::Reference(_) | SignatureToken::MutableReference(_) => {
                has_reference = true;
                counts.reference_parameters += 1
            }
            _ => (),
        }
    }
    for return_type in return_types {
        match return_type {
            SignatureToken::Reference(_) | SignatureToken::MutableReference(_) => {
                has_reference = true;
                counts.reference_return_values += 1
            }
            _ => (),
        }
    }
    if has_reference {
        counts.functions_with_reference_signatures += 1;
    }
    if !acquires.is_empty() {
        counts.functions_with_acquires += 1;
    }
    counts.acquires_annotations += acquires.len();
}

fn count_instructions(counts: &mut Counts, code: &[Bytecode]) {
    let before_reference_instruction = counts.total_reference_operations();
    for instr in code {
        count_instruction(counts, instr)
    }
    let after_reference_instruction = counts.total_reference_operations();
    if after_reference_instruction > before_reference_instruction {
        counts.functions_with_reference_operations += 1;
    }
}

fn count_instruction(counts: &mut Counts, instr: &Bytecode) {
    counts.total_instructions += 1;
    match instr {
        Bytecode::ImmBorrowLoc(_) => counts.imm_borrow_loc += 1,
        Bytecode::MutBorrowLoc(_) => counts.mut_borrow_loc += 1,

        Bytecode::ImmBorrowField(_) | Bytecode::ImmBorrowFieldGeneric(_) => {
            counts.imm_borrow_field += 1
        }
        Bytecode::MutBorrowField(_) | Bytecode::MutBorrowFieldGeneric(_) => {
            counts.mut_borrow_field += 1
        }

        Bytecode::ImmBorrowGlobal(_) | Bytecode::ImmBorrowGlobalGeneric(_) => {
            counts.imm_borrow_global += 1
        }
        Bytecode::MutBorrowGlobal(_) | Bytecode::MutBorrowGlobalGeneric(_) => {
            counts.mut_borrow_global += 1
        }

        Bytecode::FreezeRef => counts.freeze += 1,

        _ => (),
    }
}
