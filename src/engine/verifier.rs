use std::collections::VecDeque;

use super::chunk::{ExceptionHandler, FunctionProto, Opcode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackVerifyError {
    pub function_label: String,
    pub ip: usize,
    pub opcode: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlFlow {
    FallThrough,
    Jump(i32),
    CondJump {
        jump_offset: i32,
        jump_pops: i64,
        fallthrough_pops: i64,
    },
    Terminal,
}

pub fn verify_stack_balance(proto: &FunctionProto) -> Result<(), StackVerifyError> {
    verify_function(proto, function_label(proto))
}

pub fn compute_stack_depths(proto: &FunctionProto) -> Vec<Option<i64>> {
    let mut depth_at = vec![None; proto.code.len()];
    let mut mode = StackDepthMode::Compute;
    let mut worklist = VecDeque::new();
    let _ = analyze_stack_depths(proto, "", &mut depth_at, &mut worklist, &mut mode);
    depth_at
}

fn verify_function(proto: &FunctionProto, label: String) -> Result<(), StackVerifyError> {
    for (index, nested) in proto.nested_functions.iter().enumerate() {
        let nested_label = nested_label(&label, nested, index);
        verify_function(nested, nested_label)?;
    }

    let code_len = proto.code.len();
    if code_len == 0 {
        return Ok(());
    }

    let mut depth_at: Vec<Option<i64>> = vec![None; code_len];
    let mut worklist = VecDeque::new();
    let mut mode = StackDepthMode::Verify;
    analyze_stack_depths(proto, &label, &mut depth_at, &mut worklist, &mut mode)
}

enum StackDepthMode {
    Verify,
    Compute,
}

impl StackDepthMode {
    fn underflow(
        &mut self,
        label: &str,
        ip: usize,
        opcode: &Opcode,
        message: String,
    ) -> Result<(), StackVerifyError> {
        match self {
            StackDepthMode::Verify => Err(error(label, ip, opcode, message)),
            StackDepthMode::Compute => Ok(()),
        }
    }

    fn mismatch(
        &mut self,
        label: &str,
        ip: usize,
        opcode: &Opcode,
        message: String,
    ) -> Result<(), StackVerifyError> {
        match self {
            StackDepthMode::Verify => Err(error(label, ip, opcode, message)),
            StackDepthMode::Compute => Ok(()),
        }
    }

    fn out_of_range(
        &mut self,
        label: &str,
        ip: usize,
        opcode: &Opcode,
        message: String,
    ) -> Result<(), StackVerifyError> {
        match self {
            StackDepthMode::Verify => Err(error(label, ip, opcode, message)),
            StackDepthMode::Compute => Ok(()),
        }
    }
}

fn analyze_stack_depths(
    proto: &FunctionProto,
    label: &str,
    depth_at: &mut [Option<i64>],
    worklist: &mut VecDeque<usize>,
    mode: &mut StackDepthMode,
) -> Result<(), StackVerifyError> {
    if proto.code.is_empty() {
        return Ok(());
    }

    seed_depth(
        depth_at,
        worklist,
        0,
        0,
        label,
        &proto.code,
        mode,
    )?;

    for handler in &proto.handlers {
        seed_handler_entries(proto, handler, label, depth_at, worklist, mode)?;
    }

    while let Some(ip) = worklist.pop_front() {
        let Some(depth) = depth_at[ip] else {
            continue;
        };
        let opcode = &proto.code[ip];
        let (pops, pushes, flow) = stack_effect(opcode);
        if depth < pops {
            mode.underflow(label, ip, opcode, format!("stack underflow: depth {depth} < pops {pops}"))?;
            continue;
        }
        let next_depth = depth - pops + pushes;

        match flow {
            ControlFlow::FallThrough => {
                propagate_successor(
                    proto,
                    label,
                    depth_at,
                    worklist,
                    ip + 1,
                    next_depth,
                    opcode,
                    mode,
                )?;
            }
            ControlFlow::Jump(offset) => {
                let target = jump_target(ip, offset)?;
                propagate_successor(
                    proto,
                    label,
                    depth_at,
                    worklist,
                    target,
                    next_depth,
                    opcode,
                    mode,
                )?;
            }
            ControlFlow::CondJump {
                jump_offset,
                jump_pops,
                fallthrough_pops,
            } => {
                let jump_depth = depth - jump_pops + pushes;
                let fallthrough_depth = depth - fallthrough_pops + pushes;
                let jump_target = jump_target(ip, jump_offset)?;
                propagate_successor(
                    proto,
                    label,
                    depth_at,
                    worklist,
                    jump_target,
                    jump_depth,
                    opcode,
                    mode,
                )?;
                propagate_successor(
                    proto,
                    label,
                    depth_at,
                    worklist,
                    ip + 1,
                    fallthrough_depth,
                    opcode,
                    mode,
                )?;
            }
            ControlFlow::Terminal => {}
        }

        seed_handlers_for_ip(proto, ip, label, depth_at, worklist, mode)?;
    }

    Ok(())
}

fn seed_handler_entries(
    proto: &FunctionProto,
    handler: &ExceptionHandler,
    label: &str,
    depth_at: &mut [Option<i64>],
    worklist: &mut VecDeque<usize>,
    mode: &mut StackDepthMode,
) -> Result<(), StackVerifyError> {
    let Some(&try_start_depth) = depth_at.get(handler.try_start as usize).and_then(|v| v.as_ref())
    else {
        return Ok(());
    };
    if handler.catch_ip != 0 {
        seed_depth(
            depth_at,
            worklist,
            handler.catch_ip as usize,
            try_start_depth,
            label,
            &proto.code,
            mode,
        )?;
    }
    if handler.finally_ip != 0 {
        seed_depth(
            depth_at,
            worklist,
            handler.finally_ip as usize,
            try_start_depth,
            label,
            &proto.code,
            mode,
        )?;
    }
    Ok(())
}

fn seed_handlers_for_ip(
    proto: &FunctionProto,
    ip: usize,
    label: &str,
    depth_at: &mut [Option<i64>],
    worklist: &mut VecDeque<usize>,
    mode: &mut StackDepthMode,
) -> Result<(), StackVerifyError> {
    for handler in &proto.handlers {
        if handler.try_start as usize == ip {
            seed_handler_entries(proto, handler, label, depth_at, worklist, mode)?;
        }
    }
    Ok(())
}

fn seed_depth(
    depth_at: &mut [Option<i64>],
    worklist: &mut VecDeque<usize>,
    ip: usize,
    depth: i64,
    label: &str,
    code: &[Opcode],
    mode: &mut StackDepthMode,
) -> Result<(), StackVerifyError> {
    if ip >= depth_at.len() {
        return mode.out_of_range(
            label,
            code.len().saturating_sub(1),
            code.last().unwrap_or(&Opcode::Nop),
            format!("entry target out of range: {ip}"),
        );
    }
    match depth_at[ip] {
        None => {
            depth_at[ip] = Some(depth);
            worklist.push_back(ip);
        }
        Some(existing) if existing == depth => {}
        Some(existing) => {
            return mode.mismatch(
                label,
                ip,
                &code[ip],
                format!("stack depth mismatch at {ip}: {existing} vs {depth}"),
            );
        }
    }
    Ok(())
}

fn propagate_successor(
    proto: &FunctionProto,
    label: &str,
    depth_at: &mut [Option<i64>],
    worklist: &mut VecDeque<usize>,
    ip: usize,
    depth: i64,
    opcode: &Opcode,
    mode: &mut StackDepthMode,
) -> Result<(), StackVerifyError> {
    if ip >= proto.code.len() {
        return mode.out_of_range(
            label,
            proto.code.len().saturating_sub(1),
            opcode,
            format!("jump target out of range: {ip}"),
        );
    }
    match depth_at[ip] {
        None => {
            depth_at[ip] = Some(depth);
            worklist.push_back(ip);
        }
        Some(existing) if existing == depth => {}
        Some(existing) => {
            return mode.mismatch(
                label,
                ip,
                &proto.code[ip],
                format!("stack depth mismatch at {ip}: {existing} vs {depth}"),
            );
        }
    }
    Ok(())
}

fn jump_target(ip: usize, offset: i32) -> Result<usize, StackVerifyError> {
    let target = ip as i64 + 1 + offset as i64;
    if target < 0 {
        return Err(StackVerifyError {
            function_label: String::new(),
            ip,
            opcode: String::new(),
            message: format!("jump target out of range: {target}"),
        });
    }
    Ok(target as usize)
}

fn stack_effect(op: &Opcode) -> (i64, i64, ControlFlow) {
    match op {
        Opcode::LoadConst(_)
        | Opcode::LoadUndefined
        | Opcode::LoadNull
        | Opcode::LoadTrue
        | Opcode::LoadFalse
        | Opcode::LoadThis
        | Opcode::LoadNewTarget
        | Opcode::GetLocal(_)
        | Opcode::GetUpvalue(_)
        | Opcode::GetGlobal(_)
        | Opcode::GetGlobalOptional(_)
        | Opcode::DynamicImport
        | Opcode::LoadArguments
        | Opcode::MakeClosure(_)
        | Opcode::MakeObject
        | Opcode::MakeRegExp(_)
        | Opcode::GetProp
        | Opcode::GetIndex
        | Opcode::GetForInKeys
        | Opcode::GetForOfIterator
        | Opcode::GetForAwaitIterator
        | Opcode::GetProto => {
            let pushes = 1;
            let pops = match op {
                Opcode::GetProp => 2,
                Opcode::GetIndex => 2,
                Opcode::GetForInKeys => 1,
                Opcode::GetForOfIterator => 1,
                Opcode::GetForAwaitIterator => 1,
                Opcode::GetProto => 1,
                _ => 0,
            };
            (pops, pushes, ControlFlow::FallThrough)
        }
        Opcode::GetPropForCall(_) => (1, 2, ControlFlow::FallThrough),
        Opcode::GetIndexForCall => (2, 2, ControlFlow::FallThrough),
        Opcode::JumpIfTrue(_)
        | Opcode::JumpIfFalse(_)
        | Opcode::JumpIfNullish(_) => {
            let flow = match op {
                Opcode::JumpIfTrue(offset) => ControlFlow::CondJump {
                    jump_offset: *offset,
                    jump_pops: 0,
                    fallthrough_pops: 0,
                },
                Opcode::JumpIfFalse(offset) => ControlFlow::CondJump {
                    jump_offset: *offset,
                    jump_pops: 0,
                    fallthrough_pops: 0,
                },
                Opcode::JumpIfNullish(offset) => ControlFlow::CondJump {
                    jump_offset: *offset,
                    jump_pops: 0,
                    fallthrough_pops: 0,
                },
                _ => ControlFlow::FallThrough,
            };
            (0, 0, flow)
        }
        Opcode::Pop
        | Opcode::SetLocal(_)
        | Opcode::SetUpvalue(_)
        | Opcode::SetGlobal(_) => (1, 0, ControlFlow::FallThrough),
        // Unary operators transform the top of stack in place: pop 1, push 1.
        Opcode::Neg
        | Opcode::Not
        | Opcode::BitNot
        | Opcode::Typeof
        | Opcode::ToNumber
        | Opcode::Delete
        | Opcode::Void => (1, 1, ControlFlow::FallThrough),
        Opcode::Dup => (0, 1, ControlFlow::FallThrough),
        Opcode::FreshenLocal(_) => (0, 0, ControlFlow::FallThrough),
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::Div
        | Opcode::Rem
        | Opcode::Exp
        | Opcode::Eq
        | Opcode::StrictEq
        | Opcode::Ne
        | Opcode::StrictNe
        | Opcode::Lt
        | Opcode::Le
        | Opcode::Gt
        | Opcode::Ge
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::Shl
        | Opcode::Shr
        | Opcode::UShr
        | Opcode::In
        | Opcode::Instanceof => (2, 1, ControlFlow::FallThrough),
        Opcode::DeleteProp => (2, 1, ControlFlow::FallThrough),
        Opcode::DefineGetter | Opcode::DefineSetter => (3, 0, ControlFlow::FallThrough),
        Opcode::Jump(offset) => (0, 0, ControlFlow::Jump(*offset)),
        Opcode::JumpIfTruePop(offset) | Opcode::JumpIfFalsePop(offset) => {
            let jump_pops = 1;
            let fallthrough_pops = 1;
            let flow = ControlFlow::CondJump {
                jump_offset: *offset,
                jump_pops,
                fallthrough_pops,
            };
            (1, 0, flow)
        }
        Opcode::Call(argc) => ((i64::from(*argc) + 2), 1, ControlFlow::FallThrough),
        Opcode::CallSpread(argc) => ((i64::from(*argc) + 2), 1, ControlFlow::FallThrough),
        Opcode::Return | Opcode::AsyncReturn | Opcode::Throw => {
            (1, 0, ControlFlow::Terminal)
        }
        // Await/Yield suspend then resume at the next instruction: they pop the
        // awaited/yielded value and the resume pushes the resolved/sent value
        // (net 0), so execution continues — not terminal. Modelling them as
        // Terminal would leave all post-await/yield code unverified.
        Opcode::Await | Opcode::Yield => (1, 1, ControlFlow::FallThrough),
        Opcode::MakeArray(count) => (i64::from(*count), 1, ControlFlow::FallThrough),
        Opcode::SetProp | Opcode::SetIndex => (3, 0, ControlFlow::FallThrough),
        Opcode::CopyDataProperties => (2, 1, ControlFlow::FallThrough),
        Opcode::New(argc) => ((i64::from(*argc) + 1), 1, ControlFlow::FallThrough),
        Opcode::Spread => (1, 1, ControlFlow::Terminal),
        Opcode::ForOfNext => (1, 2, ControlFlow::FallThrough),
        Opcode::SetProtoOf => (2, 1, ControlFlow::FallThrough),
        Opcode::SetObjectLiteralProto => (2, 0, ControlFlow::FallThrough),
        Opcode::EnterTry(_) | Opcode::LeaveTry | Opcode::EndFinally | Opcode::Nop => {
            (0, 0, ControlFlow::FallThrough)
        }
        Opcode::GetSuperCtor => (0, 1, ControlFlow::Terminal),
    }
}

fn nested_label(parent: &str, proto: &FunctionProto, index: usize) -> String {
    match proto.name.as_deref() {
        Some(name) if !name.is_empty() => format!("{parent}::{name}#{index}"),
        _ => format!("{parent}::<nested#{index}>"),
    }
}

fn function_label(proto: &FunctionProto) -> String {
    proto
        .name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "<top-level>".to_string())
}

fn error(label: &str, ip: usize, opcode: &Opcode, message: String) -> StackVerifyError {
    StackVerifyError {
        function_label: label.to_string(),
        ip,
        opcode: format!("{opcode:?}"),
        message,
    }
}
