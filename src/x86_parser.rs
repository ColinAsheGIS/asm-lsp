use std::collections::HashMap;
use std::env::args;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::str::{self, FromStr};

use crate::types::{
    Arch, Assembler, Directive, Instruction, InstructionForm, MMXMode, NameToDirectiveMap,
    NameToInstructionMap, NameToRegisterMap, Operand, OperandType, Register, RegisterBitInfo,
    RegisterType, RegisterWidth, XMMMode, Z80Timing, Z80TimingInfo, ISA,
};

use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use quick_xml::escape::unescape;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;
use regex::Regex;
use reqwest;
use url_escape::encode_www_form_urlencoded;

/// Parse the provided XML contents and return a vector of all the instructions based on that.
/// If parsing fails, the appropriate error will be returned instead.
///
/// Current function assumes that the XML file is already read and that it's been given a reference
/// to its contents (`&str`).
///
/// # Errors
///
/// This function is highly specialized to parse a handful of files and will panic or return
/// `Err` for most mal-formed inputs
///
/// # Panics
///
/// This function is highly specialized to parse a handful of files and will panic or return
/// `Err` for most mal-formed/unexpected inputs
pub fn populate_instructions(xml_contents: &str) -> Result<Vec<Instruction>> {
    // initialise the instruction set
    let mut instructions_map = HashMap::<String, Instruction>::new();

    // iterate through the XML --------------------------------------------------------------------
    let mut reader = Reader::from_str(xml_contents);

    // ref to the instruction that's currently under construction
    let mut curr_instruction = Instruction::default();
    let mut curr_instruction_form = InstructionForm::default();
    let mut arch: Option<Arch> = None;

    debug!("Parsing instruction XML contents...");
    loop {
        match reader.read_event() {
            // start event ------------------------------------------------------------------------
            Ok(Event::Start(ref e)) => {
                match e.name() {
                    QName(b"InstructionSet") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if let Ok("name") = str::from_utf8(key.into_inner()) {
                                arch = Arch::from_str(unsafe { str::from_utf8_unchecked(&value) })
                                    .ok();
                            } else {
                                warn!("Failed to parse architecture name");
                            }
                        }
                    }
                    QName(b"Instruction") => {
                        // start of a new instruction
                        curr_instruction = Instruction::default();
                        curr_instruction.arch = arch;

                        // iterate over the attributes
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            match str::from_utf8(key.into_inner()).unwrap() {
                                "name" => {
                                    let name =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                    curr_instruction.alt_names.push(name.to_uppercase());
                                    curr_instruction.alt_names.push(name.to_lowercase());
                                    curr_instruction.name = name;
                                }
                                "summary" => {
                                    curr_instruction.summary =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                }
                                _ => {}
                            }
                        }
                    }
                    QName(b"InstructionForm") => {
                        // Read the attributes
                        //
                        // <xs:attribute name="gas-name" type="xs:string" use="required" />
                        // <xs:attribute name="go-name" type="xs:string" />
                        // <xs:attribute name="mmx-mode" type="MMXMode" />
                        // <xs:attribute name="xmm-mode" type="XMMMode" />
                        // <xs:attribute name="cancelling-inputs" type="xs:boolean" />
                        // <xs:attribute name="nacl-version" type="NaClVersion" />
                        // <xs:attribute name="nacl-zero-extends-outputs" type="xs:boolean" />

                        // new instruction form
                        curr_instruction_form = InstructionForm::default();

                        // iterate over the attributes
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            match str::from_utf8(key.into_inner()).unwrap() {
                                "gas-name" => {
                                    curr_instruction_form.gas_name = Some(String::from(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    }));
                                }
                                "go-name" => {
                                    curr_instruction_form.go_name = Some(String::from(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    }));
                                }
                                "mmx-mode" => {
                                    let value_ = value.as_ref();
                                    curr_instruction_form.mmx_mode =
                                        Some(MMXMode::from_str(unsafe {
                                            str::from_utf8_unchecked(value_)
                                        })?);
                                }
                                "xmm-mode" => {
                                    let value_ = value.as_ref();
                                    curr_instruction_form.xmm_mode =
                                        Some(XMMMode::from_str(unsafe {
                                            str::from_utf8_unchecked(value_)
                                        })?);
                                }
                                "cancelling-inputs" => match str::from_utf8(&value).unwrap() {
                                    "true" => curr_instruction_form.cancelling_inputs = Some(true),
                                    "false" => {
                                        curr_instruction_form.cancelling_inputs = Some(false);
                                    }
                                    val => {
                                        return Err(anyhow!(
                                            "Unknown value '{val}' for XML attribute cancelling inputs"
                                        ));
                                    }
                                },
                                "nacl-version" => {
                                    curr_instruction_form.nacl_version =
                                        value.as_ref().first().copied();
                                }
                                "nacl-zero-extends-outputs" => {
                                    match str::from_utf8(&value).unwrap() {
                                        "true" => {
                                            curr_instruction_form.nacl_zero_extends_outputs =
                                                Some(true);
                                        }
                                        "false" => {
                                            curr_instruction_form.nacl_zero_extends_outputs =
                                                Some(false);
                                        }
                                        val => {
                                            return Err(anyhow!(
                                                "Unknown value '{val}' for XML attribute nacl-zero-extends-outputs",
                                            ));
                                        }
                                    }
                                }
                                "z80name" => {
                                    curr_instruction_form.z80_name = Some(String::from(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    }));
                                }
                                "form" => {
                                    let value_ = unsafe { str::from_utf8_unchecked(&value) };
                                    curr_instruction_form.urls.push(format!(
                                        "https://www.zilog.com/docs/z80/z80cpu_um.pdf#{}",
                                        encode_www_form_urlencoded(value_)
                                    ));
                                    curr_instruction_form.z80_form = Some(value_.to_string());
                                }
                                _ => {}
                            }
                        }
                    }
                    // TODO
                    QName(b"Encoding") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if str::from_utf8(key.into_inner()).unwrap() == "byte" {
                                let disp_code =
                                    unsafe { str::from_utf8_unchecked(&value) }.to_string() + " ";
                                if let Some(ref mut opcodes) = curr_instruction_form.z80_opcode {
                                    opcodes.push_str(&disp_code);
                                } else {
                                    curr_instruction_form.z80_opcode = Some(disp_code);
                                }
                            }
                        }
                    }
                    _ => {} // unknown event
                }
            }
            Ok(Event::Empty(ref e)) => {
                match e.name() {
                    QName(b"ISA") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if str::from_utf8(key.into_inner()).unwrap() == "id" {
                                {
                                    curr_instruction_form.isa =
                                        Some(
                                            ISA::from_str(unsafe {
                                                str::from_utf8_unchecked(value.as_ref())
                                            })
                                            .unwrap_or_else(|_| {
                                                panic!("Unexpected ISA variant - {}", unsafe {
                                                    str::from_utf8_unchecked(&value)
                                                })
                                            }),
                                        );
                                }
                            }
                        }
                    }
                    QName(b"Operand") => {
                        let mut type_ = OperandType::k; // dummy initialisation
                        let mut extended_size = None;
                        let mut input = None;
                        let mut output = None;

                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            match str::from_utf8(key.into_inner()).unwrap() {
                                "type" => {
                                    type_ = match OperandType::from_str(str::from_utf8(&value)?) {
                                        Ok(op_type) => op_type,
                                        Err(_) => {
                                            return Err(anyhow!(
                                                "Unknown value for operand type -- Variant: {}",
                                                str::from_utf8(&value)?
                                            ));
                                        }
                                    }
                                }
                                "input" => match str::from_utf8(&value).unwrap() {
                                    "true" => input = Some(true),
                                    "false" => input = Some(false),
                                    _ => return Err(anyhow!("Unknown value for operand type")),
                                },
                                "output" => match str::from_utf8(&value).unwrap() {
                                    "true" => output = Some(true),
                                    "false" => output = Some(false),
                                    _ => return Err(anyhow!("Unknown value for operand type")),
                                },
                                "extended-size" => {
                                    extended_size = Some(
                                        str::from_utf8(value.as_ref()).unwrap().parse::<usize>()?,
                                    );
                                }
                                _ => {} // unknown event
                            }
                        }

                        curr_instruction_form.operands.push(Operand {
                            type_,
                            input,
                            output,
                            extended_size,
                        });
                    }
                    QName(b"TimingZ80") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if str::from_utf8(key.into_inner()).unwrap() == "value" {
                                let z80 = match Z80TimingInfo::from_str(unsafe {
                                    str::from_utf8_unchecked(&value)
                                }) {
                                    Ok(timing) => timing,
                                    Err(e) => return Err(anyhow!(e)),
                                };
                                if let Some(ref mut timing_entry) = curr_instruction_form.z80_timing
                                {
                                    timing_entry.z80 = z80;
                                } else {
                                    curr_instruction_form.z80_timing = Some(Z80Timing {
                                        z80,
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                    QName(b"TimingZ80M1") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if str::from_utf8(key.into_inner()).unwrap() == "value" {
                                let z80_plus_m1 = match Z80TimingInfo::from_str(unsafe {
                                    str::from_utf8_unchecked(&value)
                                }) {
                                    Ok(timing) => timing,
                                    Err(e) => return Err(anyhow!(e)),
                                };
                                if let Some(ref mut timing_entry) = curr_instruction_form.z80_timing
                                {
                                    timing_entry.z80_plus_m1 = z80_plus_m1;
                                } else {
                                    curr_instruction_form.z80_timing = Some(Z80Timing {
                                        z80_plus_m1,
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                    QName(b"TimingR800") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if str::from_utf8(key.into_inner()).unwrap() == "value" {
                                let r800 = match Z80TimingInfo::from_str(unsafe {
                                    str::from_utf8_unchecked(&value)
                                }) {
                                    Ok(timing) => timing,
                                    Err(e) => return Err(anyhow!(e)),
                                };
                                if let Some(ref mut timing_entry) = curr_instruction_form.z80_timing
                                {
                                    timing_entry.r800 = r800;
                                } else {
                                    curr_instruction_form.z80_timing = Some(Z80Timing {
                                        r800,
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                    QName(b"TimingR800Wait") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if str::from_utf8(key.into_inner()).unwrap() == "value" {
                                let r800_plus_wait = match Z80TimingInfo::from_str(unsafe {
                                    str::from_utf8_unchecked(&value)
                                }) {
                                    Ok(timing) => timing,
                                    Err(e) => return Err(anyhow!(e)),
                                };
                                if let Some(ref mut timing_entry) = curr_instruction_form.z80_timing
                                {
                                    timing_entry.r800_plus_wait = r800_plus_wait;
                                } else {
                                    curr_instruction_form.z80_timing = Some(Z80Timing {
                                        r800_plus_wait,
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                    _ => {} // unknown event
                }
            }
            // end event --------------------------------------------------------------------------
            Ok(Event::End(ref e)) => {
                match e.name() {
                    QName(b"Instruction") => {
                        // finish instruction
                        instructions_map
                            .insert(curr_instruction.name.clone(), curr_instruction.clone());
                    }
                    QName(b"InstructionForm") => {
                        curr_instruction.push_form(curr_instruction_form.clone());
                    }
                    _ => {} // unknown event
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            _ => {} // rest of events that we don't consider
        }
    }

    if let Some(Arch::X86 | Arch::X86_64) = arch {
        let x86_online_docs = get_x86_docs_url();
        let body = get_docs_body(&x86_online_docs).unwrap_or_default();
        let body_it = body.split("<td>").skip(1).step_by(2);

        // Parse this x86 page, grab the contents of the table + the URLs they are referring to
        // Regex to match:
        // <a href="./VSCATTERPF1DPS:VSCATTERPF1QPS:VSCATTERPF1DPD:VSCATTERPF1QPD.html">VSCATTERPF1QPS</a></td>
        //
        // let re = Regex::new(r"<a href=\"./(.*)">(.*)</a></td>")?;
        // let re = Regex::new(r#"<a href="\./(.*?\.html)">(.*?)</a>.*</td>"#)?;
        // let re = Regex::new(r"<a href='\/(.*?)'>(.*?)<\/a>.*<\/td>")?;
        let re = Regex::new(r"<a href='\/x86\/(.*?)'>(.*?)<\/a>.*<\/td>")?;
        for line in body_it {
            // take it step by step.. match a small portion of the line first...
            let caps = re.captures(line).unwrap();
            let url_suffix = caps.get(1).map_or("", |m| m.as_str());
            let instruction_name = caps.get(2).map_or("", |m| m.as_str());

            // add URL to the corresponding instruction
            if let Some(instruction) = instructions_map.get_mut(instruction_name) {
                instruction.url = Some(x86_online_docs.clone() + url_suffix);
            }
        }
    }

    Ok(instructions_map.into_values().collect())
}

pub fn populate_name_to_instruction_map<'instruction>(
    arch: Arch,
    instructions: &'instruction Vec<Instruction>,
    names_to_instructions: &mut NameToInstructionMap<'instruction>,
) {
    // Add the "true" names first
    for instruction in instructions {
        for name in &instruction.get_primary_names() {
            names_to_instructions.insert((arch, name), instruction);
        }
    }
    // Add alternate form names next, ensuring we don't overwrite existing entries
    for instruction in instructions {
        for name in &instruction.get_associated_names() {
            names_to_instructions
                .entry((arch, name))
                .or_insert_with(|| instruction);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::x86_parser::{get_cache_dir, populate_instructions};
    #[test]
    fn test_populate_instructions() {
        let mut server = mockito::Server::new_with_port(8080);

        let _ = server
            .mock("GET", "/x86/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body(include_str!(
                "../docs_store/instr_info_cache/x86_instr_docs.html"
            ))
            .create();

        // Need to clear the cache file (if there is one)
        // to ensure a request is made for each test call
        let mut x86_cache_path = get_cache_dir().unwrap();
        x86_cache_path.push("x86_instr_docs.html");
        if x86_cache_path.is_file() {
            std::fs::remove_file(&x86_cache_path).unwrap();
        }
        let xml_conts_x86 = include_str!("../docs_store/opcodes/raw/x86.xml");
        assert!(populate_instructions(xml_conts_x86).is_ok());

        if x86_cache_path.is_file() {
            std::fs::remove_file(&x86_cache_path).unwrap();
        }
        let xml_conts_x86_64 = include_str!("../docs_store/opcodes/raw/x86_64.xml");
        assert!(populate_instructions(xml_conts_x86_64).is_ok());

        // Clean things up so we don't have an empty cache file
        if x86_cache_path.is_file() {
            std::fs::remove_file(&x86_cache_path).unwrap();
        }
    }
}

/// Parse the provided XML contents and return a vector of all the registers based on that.
/// If parsing fails, the appropriate error will be returned instead.
///
/// Current function assumes that the XML file is already read and that it's been given a reference
/// to its contents (`&str`).
///
/// # Errors
///
/// This function is highly specialized to parse a handful of files and will panic or return
/// `Err` for most mal-formed/unexpected inputs
///
/// # Panics
///
/// This function is highly specialized to parse a handful of files and will panic or return
/// `Err` for most mal-formed/unexpected inputs
pub fn populate_registers(xml_contents: &str) -> Result<Vec<Register>> {
    let mut registers_map = HashMap::<String, Register>::new();

    // iterate through the XML --------------------------------------------------------------------
    let mut reader = Reader::from_str(xml_contents);

    // ref to the register that's currently under construction
    let mut curr_register = Register::default();
    let mut curr_bit_flag = RegisterBitInfo::default();
    let mut arch: Option<Arch> = None;

    debug!("Parsing register XML contents...");
    loop {
        match reader.read_event() {
            // start event ------------------------------------------------------------------------
            Ok(Event::Start(ref e)) => {
                match e.name() {
                    QName(b"InstructionSet") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if let Ok("name") = str::from_utf8(key.into_inner()) {
                                arch = Arch::from_str(unsafe { str::from_utf8_unchecked(&value) })
                                    .ok();
                            }
                        }
                    }
                    QName(b"Register") => {
                        // start of a new register
                        curr_register = Register::default();
                        curr_register.arch = arch;

                        // iterate over the attributes
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            match str::from_utf8(key.into_inner()).unwrap() {
                                "name" => {
                                    let name_ =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                    curr_register.alt_names.push(name_.to_uppercase());
                                    curr_register.alt_names.push(name_.to_lowercase());
                                    curr_register.name = name_;
                                }
                                "altname" => {
                                    curr_register.alt_names.push(String::from(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    }));
                                }
                                "description" => {
                                    curr_register.description = Some(String::from(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    }));
                                }
                                "type" => {
                                    curr_register.reg_type = match RegisterType::from_str(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    }) {
                                        Ok(reg) => Some(reg),
                                        _ => None,
                                    }
                                }
                                "width" => {
                                    curr_register.width = match RegisterWidth::from_str(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    }) {
                                        Ok(width) => Some(width),
                                        _ => None,
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    QName(b"Flags") => {} // it's just a wrapper...
                    // Actual flag bit info
                    QName(b"Flag") => {
                        curr_bit_flag = RegisterBitInfo::default();

                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            match str::from_utf8(key.into_inner()).unwrap() {
                                "bit" => {
                                    curr_bit_flag.bit = unsafe { str::from_utf8_unchecked(&value) }
                                        .parse::<u32>()
                                        .unwrap();
                                }
                                "label" => {
                                    curr_bit_flag.label =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                }
                                "description" => {
                                    curr_bit_flag.description =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                }
                                "pae" => {
                                    curr_bit_flag.pae =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                }
                                "longmode" => {
                                    curr_bit_flag.long_mode =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {} // unknown event
                }
            }
            // end event --------------------------------------------------------------------------
            Ok(Event::End(ref e)) => {
                match e.name() {
                    QName(b"Register") => {
                        // finish register
                        registers_map.insert(curr_register.name.clone(), curr_register.clone());
                    }
                    QName(b"Flag") => {
                        curr_register.push_flag(curr_bit_flag.clone());
                    }
                    _ => {} // unknown event
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            _ => {} // rest of events that we don't consider
        }
    }

    // TODO: Add to URL fields here?
    // https://wiki.osdev.org/CPU_Registers_x86 and https://wiki.osdev.org/CPU_Registers_x86-64
    // are less straightforward compared to the instruction set site

    Ok(registers_map.into_values().collect())
}

pub fn populate_name_to_register_map<'register>(
    arch: Arch,
    registers: &'register Vec<Register>,
    names_to_registers: &mut NameToRegisterMap<'register>,
) {
    for register in registers {
        for name in &register.get_associated_names() {
            names_to_registers.insert((arch, name), register);
        }
    }
}

/// Parse the provided XML contents and return a vector of all the directives based on that.
/// If parsing fails, the appropriate error will be returned instead.
///
/// Current function assumes that the XML file is already read and that it's been given a reference
/// to its contents (`&str`).
///
/// # Errors
///
/// This function is highly specialized to parse a handful of files and will panic or return
/// `Err` for most mal-formed/unexpected inputs
///
/// # Panics
///
/// This function is highly specialized to parse a handful of files and will panic or return
/// `Err` for most mal-formed/unexpected inputs
pub fn populate_directives(xml_contents: &str) -> Result<Vec<Directive>> {
    let mut directives_map = HashMap::<String, Directive>::new();

    // iterate through the XML --------------------------------------------------------------------
    let mut reader = Reader::from_str(xml_contents);

    // ref to the assembler directive that's currently under construction
    let mut curr_directive = Directive::default();
    let mut assembler: Option<Assembler> = None;

    debug!("Parsing directive XML contents...");
    loop {
        match reader.read_event() {
            // start event ------------------------------------------------------------------------
            Ok(Event::Start(ref e)) => {
                match e.name() {
                    QName(b"Assembler") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if let Ok("name") = str::from_utf8(key.into_inner()) {
                                assembler = Assembler::from_str(unsafe {
                                    str::from_utf8_unchecked(&value)
                                })
                                .ok();
                            }
                        }
                    }
                    QName(b"Directive") => {
                        // start of a new directive
                        curr_directive = Directive::default();
                        curr_directive.assembler = assembler;

                        // iterate over the attributes
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            match str::from_utf8(key.into_inner()).unwrap() {
                                "name" => {
                                    let name =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                    curr_directive.alt_names.push(name.to_uppercase());
                                    curr_directive.name = name;
                                }
                                "md_description" => {
                                    let description =
                                        String::from(unsafe { str::from_utf8_unchecked(&value) });
                                    curr_directive.description =
                                        unescape(&description).unwrap().to_string();
                                }
                                "deprecated" => {
                                    curr_directive.deprecated = FromStr::from_str(unsafe {
                                        str::from_utf8_unchecked(&value)
                                    })
                                    .unwrap();
                                }
                                "url_fragment" => {
                                    curr_directive.url = Some(format!(
                                        "https://sourceware.org/binutils/docs-2.41/as/{}.html",
                                        unsafe { str::from_utf8_unchecked(&value) }
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    QName(b"Signatures") => {} // it's just a wrapper...
                    QName(b"Signature") => {
                        for attr in e.attributes() {
                            let Attribute { key, value } = attr.unwrap();
                            if let Ok("sig") = str::from_utf8(key.into_inner()) {
                                let sig = String::from(unsafe { str::from_utf8_unchecked(&value) });
                                curr_directive
                                    .signatures
                                    .push(unescape(&sig).unwrap().to_string());
                            }
                        }
                    }
                    _ => {} // unknown event
                }
            }
            // end event --------------------------------------------------------------------------
            Ok(Event::End(ref e)) => {
                if let QName(b"Directive") = e.name() {
                    // finish directive
                    directives_map.insert(curr_directive.name.clone(), curr_directive.clone());
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            _ => {} // rest of events that we don't consider
        }
    }

    Ok(directives_map.into_values().collect())
}

pub fn populate_name_to_directive_map<'directive>(
    assem: Assembler,
    directives: &'directive Vec<Directive>,
    names_to_directives: &mut NameToDirectiveMap<'directive>,
) {
    for register in directives {
        for name in &register.get_associated_names() {
            names_to_directives.insert((assem, name), register);
        }
    }
}

fn get_docs_body(x86_online_docs: &str) -> Option<String> {
    // provide a URL example page -----------------------------------------------------------------
    // 1. If the cache refresh option is enabled or the cache doesn't exist, attempt to fetch the
    //    data, write it to the cache, and then use it
    // 2. Otherwise, attempt to read the data from the cache
    // 3. If invalid data is read in, attempt to remove the cache file
    let cache_refresh = args().any(|arg| arg.contains("--cache-refresh"));
    let mut x86_cache_path = match get_cache_dir() {
        Ok(cache_path) => Some(cache_path),
        Err(e) => {
            warn!("Failed to resolve the cache file path - Error: {e}.");
            None
        }
    };

    // Attempt to append the cache file name to path and see if it is valid/ exists
    let cache_exists: bool;
    if let Some(mut path) = x86_cache_path {
        path.push("x86_instr_docs.html");
        cache_exists = matches!(path.try_exists(), Ok(true));
        x86_cache_path = Some(path);
    } else {
        cache_exists = false;
    }

    let body = if cache_refresh || !cache_exists {
        match get_x86_docs_web(x86_online_docs) {
            Ok(docs) => {
                if let Some(ref path) = x86_cache_path {
                    set_x86_docs_cache(&docs, path);
                }
                docs
            }
            Err(e) => {
                error!("Failed to fetch documentation from {x86_online_docs} - Error: {e}.");
                return None;
            }
        }
    } else if let Some(ref path) = x86_cache_path {
        match get_x86_docs_cache(path) {
            Ok(docs) => docs,
            Err(e) => {
                error!(
                    "Failed to fetch documentation from the cache: {} - Error: {e}.",
                    path.display()
                );
                return None;
            }
        }
    } else {
        error!("Failed to fetch documentation from the cache - Invalid path.");
        return None;
    };

    // try to create the iterator to check if the data is valid
    // if the body produces an empty iterator, we attempt to clear the cache
    if body.split("<td>").skip(1).step_by(2).next().is_none() {
        error!("Invalid docs contents.");
        if let Some(ref path) = x86_cache_path {
            error!("Attempting to remove the cache file {}...", path.display());
            match std::fs::remove_file(path) {
                Ok(()) => {
                    error!("Cache file removed.");
                }
                Err(e) => {
                    error!("Failed to remove the cache file - Error: {e}.",);
                }
            }
        } else {
            error!("Unable to clear the cache, invalid path.");
        }
        return None;
    }

    Some(body)
}

/// Searches for the asm-lsp cache directory. First checks for the  `ASM_LSP_CACHE_DIR`
/// environment variable. If this variable is present and points to a valid directory,
/// this path is returned. Otherwise, the function returns `~/.config/asm-lsp/`
///
/// # Errors
///
/// Returns `Err` if no directory can be found through `ASM_LSP_CACHE_DIR`, and
/// then no home directory can be found on the system
pub fn get_cache_dir() -> Result<PathBuf> {
    // first check if the appropriate environment variable is set
    if let Ok(path) = std::env::var("ASM_LSP_CACHE_DIR") {
        let path = PathBuf::from(path);
        // ensure the path is valid
        if path.is_dir() {
            return Ok(path);
        }
    }

    // If the environment variable isn't set or gives an invalid path, grab the home directory and build off of that
    let mut x86_cache_path = home::home_dir().ok_or(anyhow!("Home directory not found"))?;

    x86_cache_path.push(".cache");
    x86_cache_path.push("asm-lsp");

    // create the ~/.cache/asm-lsp directory if it's not already there
    fs::create_dir_all(&x86_cache_path)?;

    Ok(x86_cache_path)
}

#[cfg(not(test))]
fn get_x86_docs_url() -> String {
    String::from("https://www.felixcloutier.com/x86/")
}

#[cfg(test)]
fn get_x86_docs_url() -> String {
    String::from("http://127.0.0.1:8080/x86/")
}

fn get_x86_docs_web(x86_online_docs: &str) -> Result<String> {
    info!("Fetching further documentation from the web -> {x86_online_docs}...");
    // grab the info from the web
    let contents = reqwest::blocking::get(x86_online_docs)?.text()?;
    Ok(contents)
}

fn get_x86_docs_cache(x86_cache_path: &PathBuf) -> Result<String, std::io::Error> {
    info!(
        "Fetching html page containing further documentation, from the cache -> {}...",
        x86_cache_path.display()
    );
    fs::read_to_string(x86_cache_path)
}

fn set_x86_docs_cache(contents: &str, x86_cache_path: &PathBuf) {
    info!("Writing to the cache file {}...", x86_cache_path.display());
    match fs::File::create(x86_cache_path) {
        Ok(mut cache_file) => {
            info!("Created the cache file {} .", x86_cache_path.display());
            match cache_file.write_all(contents.as_bytes()) {
                Ok(()) => {
                    info!("Populated the cache.");
                }
                Err(e) => {
                    error!(
                        "Failed to write to the cache file {} - Error: {e}.",
                        x86_cache_path.display()
                    );
                }
            }
        }
        Err(e) => {
            error!(
                "Failed to create the cache file {} - Error: {e}.",
                x86_cache_path.display()
            );
        }
    }
}
