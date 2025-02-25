#[cfg(test)]
mod tests {
    use core::panic;
    use std::{collections::HashMap, str::FromStr};

    use anyhow::Result;
    use lsp_textdocument::FullTextDocument;
    use lsp_types::{
        CompletionContext, CompletionItem, CompletionItemKind, CompletionParams,
        CompletionTriggerKind, HoverContents, HoverParams, MarkupContent, MarkupKind,
        PartialResultParams, Position, TextDocumentIdentifier, TextDocumentPositionParams, Uri,
        WorkDoneProgressParams,
    };
    use tree_sitter::Parser;

    use crate::{
        get_comp_resp, get_completes, get_hover_resp, get_word_from_pos_params,
        instr_filter_targets, populate_directives, populate_instructions,
        populate_name_to_directive_map, populate_name_to_instruction_map,
        populate_name_to_register_map, populate_registers, x86_parser::get_cache_dir, Arch,
        Assembler, Assemblers, Directive, Instruction, InstructionSets, NameToDirectiveMap,
        NameToInstructionMap, NameToRegisterMap, Register, TargetConfig, TreeEntry,
    };

    #[derive(Debug)]
    struct GlobalInfo {
        x86_instructions: Vec<Instruction>,
        x86_64_instructions: Vec<Instruction>,
        x86_registers: Vec<Register>,
        x86_64_registers: Vec<Register>,
        z80_instructions: Vec<Instruction>,
        z80_registers: Vec<Register>,
        gas_directives: Vec<Directive>,
    }

    #[derive(Debug)]
    struct GlobalVars<'a> {
        names_to_instructions: NameToInstructionMap<'a>,
        names_to_registers: NameToRegisterMap<'a>,
        names_to_directives: NameToDirectiveMap<'a>,
        instr_completion_items: Vec<CompletionItem>,
        reg_completion_items: Vec<CompletionItem>,
        directive_completion_items: Vec<CompletionItem>,
    }

    impl GlobalInfo {
        fn new() -> Self {
            Self {
                x86_instructions: Vec::new(),
                x86_64_instructions: Vec::new(),
                x86_registers: Vec::new(),
                x86_64_registers: Vec::new(),
                z80_instructions: Vec::new(),
                z80_registers: Vec::new(),
                gas_directives: Vec::new(),
            }
        }
    }

    impl GlobalVars<'_> {
        fn new() -> Self {
            Self {
                names_to_instructions: NameToInstructionMap::new(),
                names_to_registers: NameToRegisterMap::new(),
                names_to_directives: NameToDirectiveMap::new(),
                instr_completion_items: Vec::new(),
                reg_completion_items: Vec::new(),
                directive_completion_items: Vec::new(),
            }
        }
    }

    fn init_global_info(config: Option<TargetConfig>) -> Result<GlobalInfo> {
        let mut info = GlobalInfo::new();

        let target_config = config.unwrap_or(TargetConfig {
            version: "0.1".to_string(),
            assemblers: Assemblers {
                gas: true,
                go: true,
                z80: true,
            },
            instruction_sets: InstructionSets {
                x86: true,
                x86_64: true,
                z80: true,
            },
        });

        info.x86_instructions = {
            let x86_instrs = include_bytes!("../docs_store/opcodes/serialized/x86");
            bincode::deserialize::<Vec<Instruction>>(x86_instrs)?
                .into_iter()
                .map(|instruction| {
                    // filter out assemblers by user config
                    instr_filter_targets(&instruction, &target_config)
                })
                .filter(|instruction| !instruction.forms.is_empty())
                .collect()
        };

        info.x86_64_instructions = {
            let x86_64_instrs = include_bytes!("../docs_store/opcodes/serialized/x86_64");
            bincode::deserialize::<Vec<Instruction>>(x86_64_instrs)?
                .into_iter()
                .map(|instruction| {
                    // filter out assemblers by user config
                    instr_filter_targets(&instruction, &target_config)
                })
                .filter(|instruction| !instruction.forms.is_empty())
                .collect()
        };

        info.z80_instructions = {
            let z80_instrs = include_bytes!("../docs_store/opcodes/serialized/z80");
            bincode::deserialize::<Vec<Instruction>>(z80_instrs)?
                .into_iter()
                .map(|instruction| {
                    // filter out assemblers by user config
                    instr_filter_targets(&instruction, &target_config)
                })
                .filter(|instruction| !instruction.forms.is_empty())
                .collect()
        };

        info.x86_registers = {
            let regs_x86 = include_bytes!("../docs_store/registers/serialized/x86");
            bincode::deserialize(regs_x86)?
        };

        info.x86_64_registers = {
            let regs_x86_64 = include_bytes!("../docs_store/registers/serialized/x86_64");
            bincode::deserialize(regs_x86_64)?
        };

        info.z80_registers = {
            let regs_z80 = include_bytes!("../docs_store/registers/serialized/z80");
            bincode::deserialize(regs_z80)?
        };

        info.gas_directives = {
            let gas_dirs = include_bytes!("../docs_store/directives/serialized/gas");
            bincode::deserialize(gas_dirs)?
        };

        return Ok(info);
    }

    fn init_test_store<'a>(info: &'a GlobalInfo) -> Result<GlobalVars<'a>> {
        let mut store = GlobalVars::new();

        let mut x86_cache_path = get_cache_dir().unwrap();
        x86_cache_path.push("x86_instr_docs.html");
        if x86_cache_path.is_file() {
            std::fs::remove_file(&x86_cache_path).unwrap();
        }

        populate_name_to_instruction_map(
            Arch::X86,
            &info.x86_instructions,
            &mut store.names_to_instructions,
        );

        populate_name_to_instruction_map(
            Arch::X86_64,
            &info.x86_64_instructions,
            &mut store.names_to_instructions,
        );

        populate_name_to_instruction_map(
            Arch::Z80,
            &info.z80_instructions,
            &mut store.names_to_instructions,
        );

        populate_name_to_register_map(
            Arch::X86,
            &info.x86_registers,
            &mut store.names_to_registers,
        );

        populate_name_to_register_map(
            Arch::X86_64,
            &info.x86_64_registers,
            &mut store.names_to_registers,
        );

        populate_name_to_register_map(
            Arch::Z80,
            &info.z80_registers,
            &mut store.names_to_registers,
        );

        populate_name_to_directive_map(
            Assembler::Gas,
            &info.gas_directives,
            &mut store.names_to_directives,
        );

        store.instr_completion_items = get_completes(
            &store.names_to_instructions,
            Some(CompletionItemKind::OPERATOR),
        );

        store.reg_completion_items = get_completes(
            &store.names_to_registers,
            Some(CompletionItemKind::VARIABLE),
        );

        store.directive_completion_items = get_completes(
            &store.names_to_directives,
            Some(CompletionItemKind::OPERATOR),
        );

        return Ok(store);
    }

    fn test_hover(source: &str, expected: &str) {
        let info = init_global_info(None).expect("Failed to load info");
        let globals = init_test_store(&info).expect("Failed to initialize test store");

        let source_code = source.replace("<cursor>", "");
        let curr_doc = Some(FullTextDocument::new(
            "asm".to_string(),
            1,
            source_code.clone(),
        ));

        let mut position: Option<Position> = None;
        for (line_num, line) in source.lines().enumerate() {
            if let Some((idx, _)) = line.match_indices("<cursor>").next() {
                position = Some(Position {
                    line: line_num as u32,
                    character: idx as u32,
                });
                break;
            }
        }

        let pos_params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Uri::from_str("file://").unwrap(),
            },
            position: position.expect("No <cursor> marker found"),
        };

        let hover_params = HoverParams {
            text_document_position_params: pos_params.clone(),
            work_done_progress_params: WorkDoneProgressParams {
                work_done_token: None,
            },
        };

        let (word, file_word) = if let Some(ref doc) = curr_doc {
            (
                // get the word under the cursor
                get_word_from_pos_params(doc, &pos_params, ""),
                // treat the word under the cursor as a filename and grab it as well
                get_word_from_pos_params(doc, &pos_params, "."),
            )
        } else {
            panic!("No document");
        };

        let resp = get_hover_resp(
            &hover_params,
            &word,
            &file_word,
            &globals.names_to_instructions,
            &globals.names_to_registers,
            &globals.names_to_directives,
            &HashMap::new(),
        )
        .unwrap();

        if let HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: resp_text,
        }) = resp.contents
        {
            let cleaned = resp_text.replace("\n\n\n", "\n\n"); // not sure what's going on here...
            assert_eq!(expected, cleaned);
        } else {
            panic!("Invalid hover response contents: {:?}", resp.contents);
        }
    }

    fn test_autocomplete(
        source: &str,
        expected_kind: CompletionItemKind,
        trigger_kind: CompletionTriggerKind,
        trigger_character: Option<String>,
    ) {
        let info = init_global_info(None).expect("Failed to load info");
        let globals = init_test_store(&info).expect("Failed to initialize test store");

        let source_code = source.replace("<cursor>", "");

        let mut parser = Parser::new();
        parser.set_language(tree_sitter_asm::language()).unwrap();
        let tree = parser.parse(&source_code, None);
        let mut tree_entry = TreeEntry { tree, parser };

        let mut position: Option<Position> = None;
        for (line_num, line) in source.lines().enumerate() {
            if let Some((idx, _)) = line.match_indices("<cursor>").next() {
                position = Some(Position {
                    line: line_num as u32,
                    character: idx as u32,
                });
                break;
            }
        }

        let pos_params = TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Uri::from_str("file://").unwrap(),
            },
            position: position.expect("No <cursor> marker found"),
        };

        let comp_ctx = CompletionContext {
            trigger_kind,
            trigger_character,
        };

        let params = CompletionParams {
            text_document_position: pos_params,
            work_done_progress_params: WorkDoneProgressParams {
                work_done_token: None,
            },
            partial_result_params: PartialResultParams {
                partial_result_token: None,
            },
            context: Some(comp_ctx),
        };

        let resp = get_comp_resp(
            &source_code,
            &mut tree_entry,
            &params,
            &globals.instr_completion_items,
            &globals.directive_completion_items,
            &globals.reg_completion_items,
        )
        .unwrap();

        // - We currently have a very course-grained approach to completions,
        // - We just send all of the appropriate items (e.g. all instrucitons, all
        // registers, or all directives) and let the editor's lsp client sort out
        // which to display/ in what order
        // - Given this, we won't check for equality for all of the expected items,
        // but instead just that
        //      1) There are some items
        //      2) Said items are of the right type
        // NOTE: Both instructions and directives use the OPERATOR complection type,
        // so another means of verification should be added here
        assert!(!resp.items.is_empty());
        for comp in &resp.items {
            assert!(comp.kind == Some(expected_kind));
        }
    }

    fn test_register_autocomplete(
        source: &str,
        trigger_kind: CompletionTriggerKind,
        trigger_character: Option<String>,
    ) {
        let expected_kind = CompletionItemKind::VARIABLE;
        test_autocomplete(source, expected_kind, trigger_kind, trigger_character);
    }

    fn test_instruction_autocomplete(
        source: &str,
        trigger_kind: CompletionTriggerKind,
        trigger_character: Option<String>,
    ) {
        let expected_kind = CompletionItemKind::OPERATOR;
        test_autocomplete(source, expected_kind, trigger_kind, trigger_character);
    }

    fn test_directive_autocomplete(
        source: &str,
        trigger_kind: CompletionTriggerKind,
        trigger_character: Option<String>,
    ) {
        let expected_kind = CompletionItemKind::OPERATOR;
        test_autocomplete(source, expected_kind, trigger_kind, trigger_character);
    }

    #[test]
    fn handle_autocomplete_x86_x86_64_it_provides_instr_comps_one_character_start() {
        test_instruction_autocomplete("s<cursor>", CompletionTriggerKind::INVOKED, None);
    }

    #[test]
    fn handle_autocomplete_x86_x86_64_it_provides_reg_comps_after_percent_symbol() {
        test_register_autocomplete(
            "pushq %<cursor>",
            CompletionTriggerKind::TRIGGER_CHARACTER,
            Some("%".to_string()),
        );
    }
    #[test]
    fn handle_autocomplete_x86_x86_64_it_provides_reg_comps_in_existing_reg_arg_1() {
        test_register_autocomplete("pushq %rb<cursor>", CompletionTriggerKind::INVOKED, None);
    }
    #[test]
    fn handle_autocomplete_x86_x86_64_it_provides_reg_comps_in_existing_reg_arg_2() {
        test_register_autocomplete(
            "	movq	%rs<cursor>, %rbp",
            CompletionTriggerKind::INVOKED,
            None,
        );
    }
    #[test]
    fn handle_autocomplete_x86_x86_64_it_provides_reg_comps_in_existing_reg_arg_3() {
        test_register_autocomplete(
            "	movq	%rsp, %rb<cursor>",
            CompletionTriggerKind::INVOKED,
            None,
        );
    }
    #[test]
    fn handle_autocomplete_x86_x86_64_it_provides_reg_comps_in_existing_offset_arg() {
        test_register_autocomplete(
            "	movl	%edi, -20(%r<cursor>)",
            CompletionTriggerKind::INVOKED,
            None,
        );
    }
    #[test]
    fn handle_autocomplete_x86_x86_64_it_provides_reg_comps_in_existing_relative_addressing_arg() {
        test_register_autocomplete(
            "	leaq	_ZSt4cout(%ri<cursor>), %rdi",
            CompletionTriggerKind::INVOKED,
            None,
        );
    }

    #[test]
    fn handle_hover_x86_x86_64_it_provides_instr_info_no_args() {
        test_hover(
            "<cursor>MOVLPS",
            "MOVLPS [x86]
Move Low Packed Single-Precision Floating-Point Values

## Forms

- *GAS*: movlps | *GO*: MOVLPS | *XMM*: SSE | *ISA*: SSE

  + [xmm]    input = true   output = true
  + [m64]    input = true   output = false
- *GAS*: movlps | *GO*: MOVLPS | *XMM*: SSE | *ISA*: SSE

  + [m64]    input = false  output = true
  + [xmm]    input = true   output = false

More info: https://www.felixcloutier.com/x86/movlps

MOVLPS [x86-64]
Move Low Packed Single-Precision Floating-Point Values

## Forms

- *GAS*: movlps | *GO*: MOVLPS | *XMM*: SSE | *ISA*: SSE

  + [xmm]    input = true   output = true
  + [m64]    input = true   output = false
- *GAS*: movlps | *GO*: MOVLPS | *XMM*: SSE | *ISA*: SSE

  + [m64]    input = false  output = true
  + [xmm]    input = true   output = false

More info: https://www.felixcloutier.com/x86/movlps",
        );
    }
    #[test]
    fn handle_hover_x86_x86_64_it_provides_instr_info_one_reg_arg() {
        test_hover(
            "push<cursor>q	%rbp",
            "PUSH [x86]
Push Value Onto the Stack

## Forms

- *GAS*: pushq

  + [imm8]   extended-size = 4
- *GAS*: pushq

  + [imm32]
- *GAS*: pushw | *GO*: PUSHW

  + [r16]    input = true   output = false
- *GAS*: pushl | *GO*: PUSHL

  + [r32]    input = true   output = false
- *GAS*: pushw | *GO*: PUSHW

  + [m16]    input = true   output = false
- *GAS*: pushl | *GO*: PUSHL

  + [m32]    input = true   output = false

More info: https://www.felixcloutier.com/x86/push

PUSH [x86-64]
Push Value Onto the Stack

## Forms

- *GAS*: pushq | *GO*: PUSHQ

  + [imm8]   extended-size = 8
- *GAS*: pushq | *GO*: PUSHQ

  + [imm32]  extended-size = 8
- *GAS*: pushw | *GO*: PUSHW

  + [r16]    input = true   output = false
- *GAS*: pushq | *GO*: PUSHQ

  + [r64]    input = true   output = false
- *GAS*: pushw | *GO*: PUSHW

  + [m16]    input = true   output = false
- *GAS*: pushq | *GO*: PUSHQ

  + [m64]    input = true   output = false

More info: https://www.felixcloutier.com/x86/push",
        );
    }
    #[test]
    fn handle_hover_x86_x86_64_it_provides_instr_info_two_reg_args() {
        test_hover(
            "	m<cursor>ovq	%rsp, %rbp",
            "MOVQ [x86]
Move Quadword

## Forms

- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [mm]     input = false  output = true
  + [mm]     input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [mm]     input = false  output = true
  + [m64]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [xmm]    input = false  output = true
  + [xmm]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [xmm]    input = false  output = true
  + [m64]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [m64]    input = false  output = true
  + [mm]     input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [m64]    input = false  output = true
  + [xmm]    input = true   output = false

More info: https://www.felixcloutier.com/x86/movq

MOVQ [x86-64]
Move Quadword

## Forms

- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [r64]    input = false  output = true
  + [mm]     input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [r64]    input = false  output = true
  + [xmm]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [mm]     input = false  output = true
  + [r64]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [mm]     input = false  output = true
  + [mm]     input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [mm]     input = false  output = true
  + [m64]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [xmm]    input = false  output = true
  + [r64]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [xmm]    input = false  output = true
  + [xmm]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [xmm]    input = false  output = true
  + [m64]    input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *MMX*: MMX | *ISA*: MMX

  + [m64]    input = false  output = true
  + [mm]     input = true   output = false
- *GAS*: movq | *GO*: MOVQ | *XMM*: SSE | *ISA*: SSE2

  + [m64]    input = false  output = true
  + [xmm]    input = true   output = false

More info: https://www.felixcloutier.com/x86/movq",
        );
    }

    #[test]
    fn handle_hover_x86_x86_64_it_provides_reg_info_normal() {
        test_hover(
            "	pushq	%r<cursor>bp",
            "RBP [x86]
Stack Base Pointer

Type: General Purpose Register
Width: 64 bits

RBP [x86-64]
Base Pointer (meant for stack frames)

Type: General Purpose Register
Width: 64 bits",
        );
    }
    #[test]
    fn handle_hover_x86_x86_64_it_provides_reg_info_offset() {
        test_hover(
            "	movl	%edi, -20(%r<cursor>bp)",
            "RBP [x86]
Stack Base Pointer

Type: General Purpose Register
Width: 64 bits

RBP [x86-64]
Base Pointer (meant for stack frames)

Type: General Purpose Register
Width: 64 bits",
        );
    }
    #[test]
    fn handle_hover_x86_x86_64_it_provies_reg_info_relative_addressing() {
        test_hover(
            "	leaq	_ZSt4cout(%<cursor>rip), %rdi",
            "RIP [x86]
Instruction Pointer

Type: Pointer Register
Width: 64 bits

RIP [x86-64]
Instruction Pointer. Can only be used in RIP-relative addressing.

Type: Pointer Register
Width: 64 bits",
        );
    }

    #[test]
    fn handle_autocomplete_gas_it_provides_directive_completes_1() {
        test_directive_autocomplete("	.fi<cursor>", CompletionTriggerKind::INVOKED, None);
    }
    #[test]
    fn handle_autocomplete_gas_it_provides_directive_completes_2() {
        test_directive_autocomplete(
            r#"	.fil<cursor>	"a.cpp""#,
            CompletionTriggerKind::INVOKED,
            None,
        );
    }
    #[test]
    fn handle_autocomplete_gas_it_provides_directive_completes_3() {
        test_directive_autocomplete(
            ".<cursor>",
            CompletionTriggerKind::TRIGGER_CHARACTER,
            Some(".".to_string()),
        );
    }

    #[test]
    fn handle_hover_gas_it_provides_directive_info_1() {
        test_hover(r#"	.f<cursor>ile	"a.cpp"#, ".file [Gas]
This version of the `.file` directive tells `as` that we are about to start a new logical file. When emitting DWARF2 line number information, `.file` assigns filenames to the `.debug_line` file name table.

- .file *string*
- .file *fileno filename*

More info: https://sourceware.org/binutils/docs-2.41/as/File.html",
            );
    }
    #[test]
    fn handle_hover_gas_it_provides_directive_info_2() {
        test_hover(".<cursor>text", ".text [Gas]
Tells *as* to assemble the following statements onto the end of the text subsection numbered *subsection*, which is an absolute expression. If *subsection* is omitted, subsection number zero is used.

- .text *subsection*

More info: https://sourceware.org/binutils/docs-2.41/as/Text.html",
            );
    }
    #[test]
    fn handle_hover_gas_it_provides_directive_info_3() {
        test_hover("	.glob<cursor>l	main", ".globl [Gas]
`.globl` makes the symbol visible to `ld`. If you define symbol in your partial program, its value is made available to other partial programs that are linked with it.

- .globl *symbol*

More info: https://sourceware.org/binutils/docs-2.41/as/Global.html",
            );
    }

    #[test]
    fn handle_hover_it_demangles_cpp_1() {
        test_hover("	call	<cursor>_ZStlsISt11char_traitsIcEERSt13basic_ostreamIcT_ES5_PKc@PLT",
            "std::basic_ostream<char, std::char_traits<char> >& std::operator<< <std::char_traits<char> >(std::basic_ostream<char, std::char_traits<char> >&, char const*)",
            );
    }
    #[test]
    fn handle_hover_it_demangles_cpp_2() {
        test_hover("	leaq	_ZSt4c<cursor>out(%rip), %rdi", "std::cout");
    }
    #[test]
    fn handle_hover_it_demangles_cpp_3() {
        test_hover("	movq	_ZSt4endlIcSt<cursor>11char_traitsIcEERSt13basic_ostreamIT_T0_ES6_@GOTPCREL(%rip), %rax",
        "std::basic_ostream<char, std::char_traits<char> >& std::endl<char, std::char_traits<char> >(std::basic_ostream<char, std::char_traits<char> >&)",
            );
    }

    #[test]
    fn handle_autocomplete_z80_it_provides_instr_comps_one_character_start() {
        test_instruction_autocomplete("L<cursor>", CompletionTriggerKind::INVOKED, None);
    }

    #[test]
    fn handle_autocomplete_z80_it_provides_reg_comps_after_one_character() {
        test_register_autocomplete(
            "pushq %<cursor>",
            CompletionTriggerKind::TRIGGER_CHARACTER,
            Some("%".to_string()),
        );
    }
    #[test]
    fn handle_autocomplete_z80_it_provides_reg_comps_in_existing_reg_arg_1() {
        test_register_autocomplete("LD A<cursor>", CompletionTriggerKind::INVOKED, None);
    }
    #[test]
    fn handle_autocomplete_z80_it_provides_reg_comps_in_existing_reg_arg_2() {
        test_register_autocomplete(
            "        LD H<cursor>, DATA     ;STARTING ADDRESS OF DATA STRING",
            CompletionTriggerKind::INVOKED,
            None,
        );
    }
    #[test]
    fn handle_autocomplete_z80_it_provides_reg_comps_in_existing_reg_arg_3() {
        test_register_autocomplete(
            "        CP (H<cursor>)         ;COMPARE MEMORY CONTENTS WITH",
            CompletionTriggerKind::INVOKED,
            None,
        );
    }

    #[test]
    fn handle_hover_z80_it_provides_instr_info_no_args() {
        test_hover("        LD<cursor>I             ;MOVE CHARACTER (HL) to (DE)",
"ldi [z80]
LoaD and Increment. Copies the byte pointed to by HL to the address pointed to by DE, then adds 1 to DE and HL and subtracts 1 from BC. P/V is set to (BC!=0), i.e. set when non zero.

## Forms

- *Z80*: LDI

  + Z80: 16, Z80 + M1: 18, R800: 4, R800 + Wait: 18
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LDI
",
            );
    }
    #[test]
    fn handle_hover_z80_it_provides_instr_info_one_reg_arg() {
        test_hover("        CP<cursor> (HL)         ;COMPARE MEMORY CONTENTS WITH",
            "cp [z80]
ComPare. Sets the flags as if a SUB was performed but does not perform it. Legal combinations are the same as SUB. This is commonly used to set the flags to perform an equality or greater/less test.

## Forms

- *Z80*: CP (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#CP%20%28HL%29

- *Z80*: CP (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#CP%20%28IX%2Bo%29

- *Z80*: CP (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#CP%20%28IY%2Bo%29

- *Z80*: CP n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#CP%20n

- *Z80*: CP r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#CP%20r

- *Z80*: CP IXp

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#CP%20IXp

- *Z80*: CP IYq

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#CP%20IYq
",
            );
    }
    #[test]
    fn handle_hover_z80_it_provides_instr_info_two_reg_args() {
        test_hover("        L<cursor>D HL, DATA     ;STARTING ADDRESS OF DATA STRING",
"ld [z80]
LoaD. The basic data load/transfer instruction. Transfers data from the location specified by the second argument, to the location specified by the first.

## Forms

- *Z80*: LD (BC), A

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28BC%29%2C%20A

- *Z80*: LD (DE), A

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28DE%29%2C%20A

- *Z80*: LD (HL), n

  + Z80: 10, Z80 + M1: 11, R800: 3, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28HL%29%2C%20n

- *Z80*: LD (HL), r

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28HL%29%2C%20r

- *Z80*: LD (IX+o), n

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28IX%2Bo%29%2C%20n

- *Z80*: LD (IX+o), r

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28IX%2Bo%29%2C%20r

- *Z80*: LD (IY+o), n

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28IY%2Bo%29%2C%20n

- *Z80*: LD (IY+o), r

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28IY%2Bo%29%2C%20r

- *Z80*: LD (nn), A

  + Z80: 13, Z80 + M1: 14, R800: 4, R800 + Wait: 14
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28nn%29%2C%20A

- *Z80*: LD (nn), BC

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28nn%29%2C%20BC

- *Z80*: LD (nn), DE

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28nn%29%2C%20DE

- *Z80*: LD (nn), HL

  + Z80: 16, Z80 + M1: 17, R800: 5, R800 + Wait: 17
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28nn%29%2C%20HL

- *Z80*: LD (nn), IX

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28nn%29%2C%20IX

- *Z80*: LD (nn), IY

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28nn%29%2C%20IY

- *Z80*: LD (nn), SP

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20%28nn%29%2C%20SP

- *Z80*: LD A, (BC)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20%28BC%29

- *Z80*: LD A, (DE)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20%28DE%29

- *Z80*: LD A, (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20%28HL%29

- *Z80*: LD A, (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20%28IX%2Bo%29

- *Z80*: LD A, (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20%28IY%2Bo%29

- *Z80*: LD A, (nn)

  + Z80: 13, Z80 + M1: 14, R800: 4, R800 + Wait: 14
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20%28nn%29

- *Z80*: LD A, n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20n

- *Z80*: LD A, r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20r

- *Z80*: LD A, IXp

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20IXp

- *Z80*: LD A, IYq

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20IYq

- *Z80*: LD A, I

  + Z80: 9, Z80 + M1: 11, R800: 2, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20I

- *Z80*: LD A, R

  + Z80: 9, Z80 + M1: 11, R800: 2, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20A%2C%20R

- *Z80*: LD B, (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20B%2C%20%28HL%29

- *Z80*: LD B, (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20B%2C%20%28IX%2Bo%29

- *Z80*: LD B, (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20B%2C%20%28IY%2Bo%29

- *Z80*: LD B, n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20B%2C%20n

- *Z80*: LD B, r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20B%2C%20r

- *Z80*: LD B, IXp

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20B%2C%20IXp

- *Z80*: LD B, IYq

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20B%2C%20IYq

- *Z80*: LD BC, (nn)

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20BC%2C%20%28nn%29

- *Z80*: LD BC, nn

  + Z80: 10, Z80 + M1: 11, R800: 3, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20BC%2C%20nn

- *Z80*: LD C, (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20C%2C%20%28HL%29

- *Z80*: LD C, (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20C%2C%20%28IX%2Bo%29

- *Z80*: LD C, (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20C%2C%20%28IY%2Bo%29

- *Z80*: LD C, n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20C%2C%20n

- *Z80*: LD C, r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20C%2C%20r

- *Z80*: LD C, IXp

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20C%2C%20IXp

- *Z80*: LD C, IYq

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20C%2C%20IYq

- *Z80*: LD D, (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20D%2C%20%28HL%29

- *Z80*: LD D, (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20D%2C%20%28IX%2Bo%29

- *Z80*: LD D, (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20D%2C%20%28IY%2Bo%29

- *Z80*: LD D, n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20D%2C%20n

- *Z80*: LD D, r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20D%2C%20r

- *Z80*: LD D, IXp

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20D%2C%20IXp

- *Z80*: LD D, IYq

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20D%2C%20IYq

- *Z80*: LD DE, (nn)

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20DE%2C%20%28nn%29

- *Z80*: LD DE, nn

  + Z80: 10, Z80 + M1: 11, R800: 3, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20DE%2C%20nn

- *Z80*: LD E, (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20E%2C%20%28HL%29

- *Z80*: LD E, (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20E%2C%20%28IX%2Bo%29

- *Z80*: LD E, (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20E%2C%20%28IY%2Bo%29

- *Z80*: LD E, n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20E%2C%20n

- *Z80*: LD E, r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20E%2C%20r

- *Z80*: LD E, IXp

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20E%2C%20IXp

- *Z80*: LD E, IYq

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20E%2C%20IYq

- *Z80*: LD H, (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20H%2C%20%28HL%29

- *Z80*: LD H, (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20H%2C%20%28IX%2Bo%29

- *Z80*: LD H, (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20H%2C%20%28IY%2Bo%29

- *Z80*: LD H, n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20H%2C%20n

- *Z80*: LD H, r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20H%2C%20r

- *Z80*: LD HL, (nn)

  + Z80: 16, Z80 + M1: 17, R800: 5, R800 + Wait: 17
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20HL%2C%20%28nn%29

- *Z80*: LD HL, nn

  + Z80: 10, Z80 + M1: 11, R800: 3, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20HL%2C%20nn

- *Z80*: LD I, A

  + Z80: 9, Z80 + M1: 11, R800: 2, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20I%2C%20A

- *Z80*: LD IX, (nn)

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IX%2C%20%28nn%29

- *Z80*: LD IX, nn

  + Z80: 14, Z80 + M1: 16, R800: 4, R800 + Wait: 16
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IX%2C%20nn

- *Z80*: LD IXh, n

  + Z80: 11, Z80 + M1: 13, R800: 3, R800 + Wait: 13
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IXh%2C%20n

- *Z80*: LD IXh, p

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IXh%2C%20p

- *Z80*: LD IXl, n

  + Z80: 11, Z80 + M1: 13, R800: 3, R800 + Wait: 13
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IXl%2C%20n

- *Z80*: LD IXl, p

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IXl%2C%20p

- *Z80*: LD IY, (nn)

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IY%2C%20%28nn%29

- *Z80*: LD IY, nn

  + Z80: 14, Z80 + M1: 16, R800: 4, R800 + Wait: 16
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IY%2C%20nn

- *Z80*: LD IYh, n

  + Z80: 11, Z80 + M1: 13, R800: 3, R800 + Wait: 13
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IYh%2C%20n

- *Z80*: LD IYh, q

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IYh%2C%20q

- *Z80*: LD IYl, n

  + Z80: 11, Z80 + M1: 13, R800: 3, R800 + Wait: 13
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IYl%2C%20n

- *Z80*: LD IYl, q

  + Z80: 8, Z80 + M1: 10, R800: 2, R800 + Wait: 10
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20IYl%2C%20q

- *Z80*: LD L, (HL)

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20L%2C%20%28HL%29

- *Z80*: LD L, (IX+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20L%2C%20%28IX%2Bo%29

- *Z80*: LD L, (IY+o)

  + Z80: 19, Z80 + M1: 21, R800: 5, R800 + Wait: 21
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20L%2C%20%28IY%2Bo%29

- *Z80*: LD L, n

  + Z80: 7, Z80 + M1: 8, R800: 2, R800 + Wait: 8
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20L%2C%20n

- *Z80*: LD L, r

  + Z80: 4, Z80 + M1: 5, R800: 1, R800 + Wait: 5
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20L%2C%20r

- *Z80*: LD R, A

  + Z80: 9, Z80 + M1: 11, R800: 2, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20R%2C%20A

- *Z80*: LD SP, (nn)

  + Z80: 20, Z80 + M1: 22, R800: 6, R800 + Wait: 22
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20SP%2C%20%28nn%29

- *Z80*: LD SP, HL

  + Z80: 6, Z80 + M1: 7, R800: 1, R800 + Wait: 7
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20SP%2C%20HL

- *Z80*: LD SP, IX

  + Z80: 10, Z80 + M1: 12, R800: 2, R800 + Wait: 12
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20SP%2C%20IX

- *Z80*: LD SP, IY

  + Z80: 10, Z80 + M1: 12, R800: 2, R800 + Wait: 12
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20SP%2C%20IY

- *Z80*: LD SP, nn

  + Z80: 10, Z80 + M1: 11, R800: 3, R800 + Wait: 11
  + More info: https://www.zilog.com/docs/z80/z80cpu_um.pdf#LD%20SP%2C%20nn
"
            );
    }

    #[test]
    fn handle_hover_z80_it_provides_reg_info_normal() {
        test_hover(
            "        LD H<cursor>L, DATA     ;STARTING ADDRESS OF DATA STRING",
            "HL [z80]
16-bit accumulator/address register or two 8-bit registers.

Width: 16 bits",
        );
    }
    #[test]
    fn handle_hover_z80_it_provides_reg_info_prime() {
        test_hover(
            "        LD B<cursor>', 132      ;MAXIMUM STRING LENGTH",
            "B [z80]
General purpose register.

Type: General Purpose Register
Width: 8 bits",
        );
    }

    #[test]
    fn serialized_x86_registers_are_up_to_date() {
        let mut cmp_map = HashMap::new();
        let x86_regs_ser = include_bytes!("../docs_store/registers/serialized/x86");
        let ser_vec = bincode::deserialize::<Vec<Register>>(x86_regs_ser).unwrap();

        let x86_regs_raw = include_str!("../docs_store/registers/raw/x86.xml");
        let mut raw_vec = populate_registers(x86_regs_raw).unwrap();

        // HACK: Windows line endings...
        for reg in raw_vec.iter_mut() {
            if let Some(descr) = &reg.description {
                reg.description = Some(descr.replace('\r', ""));
            }
        }

        for reg in ser_vec {
            *cmp_map.entry(reg.clone()).or_insert(0) += 1;
        }
        for reg in raw_vec {
            let entry = cmp_map.get_mut(&reg).unwrap();
            if *entry == 0 {
                panic!(
                    "Expected at least one more instruction entry for {:?}, but the count is 0",
                    reg
                );
            }
            *entry -= 1;
        }
        for (reg, count) in cmp_map.iter() {
            if *count != 0 {
                panic!("Expected count to be 0, found {count} for {:?}", reg);
            }
        }
    }
    #[test]
    fn serialized_x86_64_registers_are_up_to_date() {
        let mut cmp_map = HashMap::new();
        let x86_64_regs_ser = include_bytes!("../docs_store/registers/serialized/x86_64");
        let ser_vec = bincode::deserialize::<Vec<Register>>(x86_64_regs_ser).unwrap();

        let x86_64_regs_raw = include_str!("../docs_store/registers/raw/x86_64.xml");
        let mut raw_vec = populate_registers(x86_64_regs_raw).unwrap();

        // HACK: Windows line endings...
        for reg in raw_vec.iter_mut() {
            if let Some(descr) = &reg.description {
                reg.description = Some(descr.replace('\r', ""));
            }
        }

        for reg in ser_vec {
            *cmp_map.entry(reg.clone()).or_insert(0) += 1;
        }
        for reg in raw_vec {
            let entry = cmp_map.get_mut(&reg).unwrap();
            if *entry == 0 {
                panic!(
                    "Expected at least one more instruction entry for {:?}, but the count is 0",
                    reg
                );
            }
            *entry -= 1;
        }
        for (reg, count) in cmp_map.iter() {
            if *count != 0 {
                panic!("Expected count to be 0, found {count} for {:?}", reg);
            }
        }
    }
    #[test]
    fn serialized_z80_registers_are_up_to_date() {
        let mut cmp_map = HashMap::new();
        let z80_regs_ser = include_bytes!("../docs_store/registers/serialized/z80");
        let ser_vec = bincode::deserialize::<Vec<Register>>(z80_regs_ser).unwrap();

        let z80_regs_raw = include_str!("../docs_store/registers/raw/z80.xml");
        let raw_vec = populate_registers(z80_regs_raw).unwrap();

        for reg in ser_vec {
            *cmp_map.entry(reg.clone()).or_insert(0) += 1;
        }
        for reg in raw_vec {
            let entry = cmp_map.get_mut(&reg).unwrap();
            if *entry == 0 {
                panic!(
                    "Expected at least one more instruction entry for {:?}, but the count is 0",
                    reg
                );
            }
            *entry -= 1;
        }
        for (reg, count) in cmp_map.iter() {
            if *count != 0 {
                panic!("Expected count to be 0, found {count} for {:?}", reg);
            }
        }
    }
    #[test]
    fn serialized_x86_instructions_are_up_to_date() {
        let mut cmp_map = HashMap::new();
        let x86_instrs_ser = include_bytes!("../docs_store/opcodes/serialized/x86");
        let mut ser_vec = bincode::deserialize::<Vec<Instruction>>(x86_instrs_ser).unwrap();

        let x86_instrs_raw = include_str!("../docs_store/opcodes/raw/x86.xml");
        let mut raw_vec = populate_instructions(x86_instrs_raw).unwrap();

        // HACK: To work around the difference in extra info urls between testing
        // and production
        for instr in ser_vec.iter_mut() {
            instr.url = None;
        }
        for instr in raw_vec.iter_mut() {
            instr.url = None;
        }

        for instr in ser_vec {
            *cmp_map.entry(instr.clone()).or_insert(0) += 1;
        }
        for instr in raw_vec {
            let entry = cmp_map.get_mut(&instr).unwrap();
            if *entry == 0 {
                panic!(
                    "Expected at least one more instruction entry for {:?}, but the count is 0",
                    instr
                );
            }
            *entry -= 1;
        }
        for (instr, count) in cmp_map.iter() {
            if *count != 0 {
                panic!("Expected count to be 0, found {count} for {:?}", instr);
            }
        }
    }
    #[test]
    fn serialized_x86_64_instructions_are_up_to_date() {
        let mut cmp_map = HashMap::new();
        let x86_64_instrs_ser = include_bytes!("../docs_store/opcodes/serialized/x86_64");
        let mut ser_vec = bincode::deserialize::<Vec<Instruction>>(x86_64_instrs_ser).unwrap();

        let x86_64_instrs_raw = include_str!("../docs_store/opcodes/raw/x86_64.xml");
        let mut raw_vec = populate_instructions(x86_64_instrs_raw).unwrap();

        // HACK: To work around the difference in extra info urls between testing
        // and production
        for instr in ser_vec.iter_mut() {
            instr.url = None;
        }
        for instr in raw_vec.iter_mut() {
            instr.url = None;
        }

        for instr in ser_vec {
            *cmp_map.entry(instr.clone()).or_insert(0) += 1;
        }
        for instr in raw_vec {
            let entry = cmp_map.get_mut(&instr).unwrap();
            if *entry == 0 {
                panic!(
                    "Expected at least one more instruction entry for {:?}, but the count is 0",
                    instr
                );
            }
            *entry -= 1;
        }
        for (instr, count) in cmp_map.iter() {
            if *count != 0 {
                panic!("Expected count to be 0, found {count} for {:?}", instr);
            }
        }
    }
    #[test]
    fn serialized_z80_instructions_are_up_to_date() {
        let mut cmp_map = HashMap::new();
        let z80_instrs_ser = include_bytes!("../docs_store/opcodes/serialized/z80");
        let ser_vec = bincode::deserialize::<Vec<Instruction>>(z80_instrs_ser).unwrap();

        let z80_instrs_raw = include_str!("../docs_store/opcodes/raw/z80.xml");
        let raw_vec = populate_instructions(z80_instrs_raw).unwrap();

        for instr in ser_vec {
            *cmp_map.entry(instr.clone()).or_insert(0) += 1;
        }
        for instr in raw_vec {
            let entry = cmp_map.get_mut(&instr).unwrap();
            if *entry == 0 {
                panic!(
                    "Expected at least one more instruction entry for {:?}, but the count is 0",
                    instr
                );
            }
            *entry -= 1;
        }
        for (instr, count) in cmp_map.iter() {
            if *count != 0 {
                panic!("Expected count to be 0, found {count} for {:?}", instr);
            }
        }
    }
    #[test]
    fn serialized_gas_directives_are_up_to_date() {
        let mut cmp_map = HashMap::new();
        let gas_dirs_ser = include_bytes!("../docs_store/directives/serialized/gas");
        let ser_vec = bincode::deserialize::<Vec<Directive>>(gas_dirs_ser).unwrap();

        let gas_dirs_raw = include_str!("../docs_store/directives/raw/gas.xml");
        let raw_vec = populate_directives(gas_dirs_raw).unwrap();

        for dir in ser_vec {
            *cmp_map.entry(dir.clone()).or_insert(0) += 1;
        }
        for dir in raw_vec {
            let entry = cmp_map.get_mut(&dir).unwrap();
            if *entry == 0 {
                panic!(
                    "Expected at least one more instruction entry for {:?}, but the count is 0",
                    dir
                );
            }
            *entry -= 1;
        }
        for (dir, count) in cmp_map.iter() {
            if *count != 0 {
                panic!("Expected count to be 0, found {count} for {:?}", dir);
            }
        }
    }
}
