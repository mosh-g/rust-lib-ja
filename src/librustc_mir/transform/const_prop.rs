//! Propagates constants for early reporting of statically known
//! assertion failures

use rustc::hir::def::DefKind;
use rustc::mir::{
    AggregateKind, Constant, Location, Place, PlaceBase, Mir, Operand, Rvalue, Local,
    NullOp, UnOp, StatementKind, Statement, LocalKind, Static, StaticKind,
    TerminatorKind, Terminator,  ClearCrossCrate, SourceInfo, BinOp, ProjectionElem,
    SourceScope, SourceScopeLocalData, LocalDecl, Promoted,
};
use rustc::mir::visit::{
    Visitor, PlaceContext, MutatingUseContext, MutVisitor, NonMutatingUseContext,
};
use rustc::mir::interpret::{InterpError, Scalar, GlobalId, EvalResult};
use rustc::ty::{self, Instance, ParamEnv, Ty, TyCtxt};
use syntax_pos::{Span, DUMMY_SP};
use rustc::ty::subst::InternalSubsts;
use rustc_data_structures::indexed_vec::IndexVec;
use rustc::ty::layout::{
    LayoutOf, TyLayout, LayoutError,
    HasTyCtxt, TargetDataLayout, HasDataLayout,
};

use crate::interpret::{self, InterpretCx, ScalarMaybeUndef, Immediate, OpTy, ImmTy, MemoryKind};
use crate::const_eval::{
    CompileTimeInterpreter, error_to_const_error, eval_promoted, mk_eval_cx,
};
use crate::transform::{MirPass, MirSource};

pub struct ConstProp;

impl MirPass for ConstProp {
    fn run_pass<'a, 'tcx>(&self,
                          tcx: TyCtxt<'a, 'tcx, 'tcx>,
                          source: MirSource<'tcx>,
                          mir: &mut Mir<'tcx>) {
        // will be evaluated by miri and produce its errors there
        if source.promoted.is_some() {
            return;
        }

        use rustc::hir::map::blocks::FnLikeNode;
        let hir_id = tcx.hir().as_local_hir_id(source.def_id())
                              .expect("Non-local call to local provider is_const_fn");

        let is_fn_like = FnLikeNode::from_node(tcx.hir().get_by_hir_id(hir_id)).is_some();
        let is_assoc_const = match tcx.def_kind(source.def_id()) {
            Some(DefKind::AssociatedConst) => true,
            _ => false,
        };

        // Only run const prop on functions, methods, closures and associated constants
        if !is_fn_like && !is_assoc_const  {
            // skip anon_const/statics/consts because they'll be evaluated by miri anyway
            trace!("ConstProp skipped for {:?}", source.def_id());
            return
        }

        trace!("ConstProp starting for {:?}", source.def_id());

        // FIXME(oli-obk, eddyb) Optimize locals (or even local paths) to hold
        // constants, instead of just checking for const-folding succeeding.
        // That would require an uniform one-def no-mutation analysis
        // and RPO (or recursing when needing the value of a local).
        let mut optimization_finder = ConstPropagator::new(mir, tcx, source);
        optimization_finder.visit_mir(mir);

        // put back the data we stole from `mir`
        std::mem::replace(
            &mut mir.source_scope_local_data,
            optimization_finder.source_scope_local_data
        );
        std::mem::replace(
            &mut mir.promoted,
            optimization_finder.promoted
        );

        trace!("ConstProp done for {:?}", source.def_id());
    }
}

type Const<'tcx> = OpTy<'tcx>;

/// Finds optimization opportunities on the MIR.
struct ConstPropagator<'a, 'mir, 'tcx:'a+'mir> {
    ecx: InterpretCx<'a, 'mir, 'tcx, CompileTimeInterpreter<'a, 'mir, 'tcx>>,
    tcx: TyCtxt<'a, 'tcx, 'tcx>,
    source: MirSource<'tcx>,
    places: IndexVec<Local, Option<Const<'tcx>>>,
    can_const_prop: IndexVec<Local, bool>,
    param_env: ParamEnv<'tcx>,
    source_scope_local_data: ClearCrossCrate<IndexVec<SourceScope, SourceScopeLocalData>>,
    local_decls: IndexVec<Local, LocalDecl<'tcx>>,
    promoted: IndexVec<Promoted, Mir<'tcx>>,
}

impl<'a, 'b, 'tcx> LayoutOf for ConstPropagator<'a, 'b, 'tcx> {
    type Ty = Ty<'tcx>;
    type TyLayout = Result<TyLayout<'tcx>, LayoutError<'tcx>>;

    fn layout_of(&self, ty: Ty<'tcx>) -> Self::TyLayout {
        self.tcx.layout_of(self.param_env.and(ty))
    }
}

impl<'a, 'b, 'tcx> HasDataLayout for ConstPropagator<'a, 'b, 'tcx> {
    #[inline]
    fn data_layout(&self) -> &TargetDataLayout {
        &self.tcx.data_layout
    }
}

impl<'a, 'b, 'tcx> HasTyCtxt<'tcx> for ConstPropagator<'a, 'b, 'tcx> {
    #[inline]
    fn tcx<'c>(&'c self) -> TyCtxt<'c, 'tcx, 'tcx> {
        self.tcx
    }
}

impl<'a, 'mir, 'tcx> ConstPropagator<'a, 'mir, 'tcx> {
    fn new(
        mir: &mut Mir<'tcx>,
        tcx: TyCtxt<'a, 'tcx, 'tcx>,
        source: MirSource<'tcx>,
    ) -> ConstPropagator<'a, 'mir, 'tcx> {
        let param_env = tcx.param_env(source.def_id());
        let ecx = mk_eval_cx(tcx, tcx.def_span(source.def_id()), param_env);
        let can_const_prop = CanConstProp::check(mir);
        let source_scope_local_data = std::mem::replace(
            &mut mir.source_scope_local_data,
            ClearCrossCrate::Clear
        );
        let promoted = std::mem::replace(
            &mut mir.promoted,
            IndexVec::new()
        );

        ConstPropagator {
            ecx,
            tcx,
            source,
            param_env,
            can_const_prop,
            places: IndexVec::from_elem(None, &mir.local_decls),
            source_scope_local_data,
            //FIXME(wesleywiser) we can't steal this because `Visitor::super_visit_mir()` needs it
            local_decls: mir.local_decls.clone(),
            promoted,
        }
    }

    fn use_ecx<F, T>(
        &mut self,
        source_info: SourceInfo,
        f: F
    ) -> Option<T>
    where
        F: FnOnce(&mut Self) -> EvalResult<'tcx, T>,
    {
        self.ecx.tcx.span = source_info.span;
        let lint_root = match self.source_scope_local_data {
            ClearCrossCrate::Set(ref ivs) => {
                //FIXME(#51314): remove this check
                if source_info.scope.index() >= ivs.len() {
                    return None;
                }
                ivs[source_info.scope].lint_root
            },
            ClearCrossCrate::Clear => return None,
        };
        let r = match f(self) {
            Ok(val) => Some(val),
            Err(error) => {
                let diagnostic = error_to_const_error(&self.ecx, error);
                use rustc::mir::interpret::InterpError::*;
                match diagnostic.error {
                    // don't report these, they make no sense in a const prop context
                    | MachineError(_)
                    | Exit(_)
                    // at runtime these transformations might make sense
                    // FIXME: figure out the rules and start linting
                    | FunctionAbiMismatch(..)
                    | FunctionArgMismatch(..)
                    | FunctionRetMismatch(..)
                    | FunctionArgCountMismatch
                    // fine at runtime, might be a register address or sth
                    | ReadBytesAsPointer
                    // fine at runtime
                    | ReadForeignStatic
                    | Unimplemented(_)
                    // don't report const evaluator limits
                    | StackFrameLimitReached
                    | NoMirFor(..)
                    | InlineAsm
                    => {},

                    | InvalidMemoryAccess
                    | DanglingPointerDeref
                    | DoubleFree
                    | InvalidFunctionPointer
                    | InvalidBool
                    | InvalidDiscriminant(..)
                    | PointerOutOfBounds { .. }
                    | InvalidNullPointerUsage
                    | ValidationFailure(..)
                    | InvalidPointerMath
                    | ReadUndefBytes(_)
                    | DeadLocal
                    | InvalidBoolOp(_)
                    | DerefFunctionPointer
                    | ExecuteMemory
                    | Intrinsic(..)
                    | InvalidChar(..)
                    | AbiViolation(_)
                    | AlignmentCheckFailed{..}
                    | CalledClosureAsFunction
                    | VtableForArgumentlessMethod
                    | ModifiedConstantMemory
                    | ModifiedStatic
                    | AssumptionNotHeld
                    // FIXME: should probably be removed and turned into a bug! call
                    | TypeNotPrimitive(_)
                    | ReallocatedWrongMemoryKind(_, _)
                    | DeallocatedWrongMemoryKind(_, _)
                    | ReallocateNonBasePtr
                    | DeallocateNonBasePtr
                    | IncorrectAllocationInformation(..)
                    | UnterminatedCString(_)
                    | HeapAllocZeroBytes
                    | HeapAllocNonPowerOfTwoAlignment(_)
                    | Unreachable
                    | ReadFromReturnPointer
                    | GeneratorResumedAfterReturn
                    | GeneratorResumedAfterPanic
                    | ReferencedConstant
                    | InfiniteLoop
                    => {
                        // FIXME: report UB here
                    },

                    | OutOfTls
                    | TlsOutOfBounds
                    | PathNotFound(_)
                    => bug!("these should not be in rustc, but in miri's machine errors"),

                    | Layout(_)
                    | UnimplementedTraitSelection
                    | TypeckError
                    | TooGeneric
                    // these are just noise
                    => {},

                    // non deterministic
                    | ReadPointerAsBytes
                    // FIXME: implement
                    => {},

                    | Panic { .. }
                    | BoundsCheck{..}
                    | Overflow(_)
                    | OverflowNeg
                    | DivisionByZero
                    | RemainderByZero
                    => {
                        diagnostic.report_as_lint(
                            self.ecx.tcx,
                            "this expression will panic at runtime",
                            lint_root,
                            None,
                        );
                    }
                }
                None
            },
        };
        self.ecx.tcx.span = DUMMY_SP;
        r
    }

    fn eval_constant(
        &mut self,
        c: &Constant<'tcx>,
    ) -> Option<Const<'tcx>> {
        self.ecx.tcx.span = c.span;
        match self.ecx.eval_const_to_op(*c.literal, None) {
            Ok(op) => {
                Some(op)
            },
            Err(error) => {
                let err = error_to_const_error(&self.ecx, error);
                err.report_as_error(self.ecx.tcx, "erroneous constant used");
                None
            },
        }
    }

    fn eval_place(&mut self, place: &Place<'tcx>, source_info: SourceInfo) -> Option<Const<'tcx>> {
        match *place {
            Place::Base(PlaceBase::Local(loc)) => self.places[loc].clone(),
            Place::Projection(ref proj) => match proj.elem {
                ProjectionElem::Field(field, _) => {
                    trace!("field proj on {:?}", proj.base);
                    let base = self.eval_place(&proj.base, source_info)?;
                    let res = self.use_ecx(source_info, |this| {
                        this.ecx.operand_field(base, field.index() as u64)
                    })?;
                    Some(res)
                },
                // We could get more projections by using e.g., `operand_projection`,
                // but we do not even have the stack frame set up properly so
                // an `Index` projection would throw us off-track.
                _ => None,
            },
            Place::Base(
                PlaceBase::Static(box Static {kind: StaticKind::Promoted(promoted), ..})
            ) => {
                let generics = self.tcx.generics_of(self.source.def_id());
                if generics.requires_monomorphization(self.tcx) {
                    // FIXME: can't handle code with generics
                    return None;
                }
                let substs = InternalSubsts::identity_for_item(self.tcx, self.source.def_id());
                let instance = Instance::new(self.source.def_id(), substs);
                let cid = GlobalId {
                    instance,
                    promoted: Some(promoted),
                };
                // cannot use `const_eval` here, because that would require having the MIR
                // for the current function available, but we're producing said MIR right now
                let res = self.use_ecx(source_info, |this| {
                    let mir = &this.promoted[promoted];
                    eval_promoted(this.tcx, cid, mir, this.param_env)
                })?;
                trace!("evaluated promoted {:?} to {:?}", promoted, res);
                Some(res.into())
            },
            _ => None,
        }
    }

    fn eval_operand(&mut self, op: &Operand<'tcx>, source_info: SourceInfo) -> Option<Const<'tcx>> {
        match *op {
            Operand::Constant(ref c) => self.eval_constant(c),
            | Operand::Move(ref place)
            | Operand::Copy(ref place) => self.eval_place(place, source_info),
        }
    }

    fn const_prop(
        &mut self,
        rvalue: &Rvalue<'tcx>,
        place_layout: TyLayout<'tcx>,
        source_info: SourceInfo,
    ) -> Option<Const<'tcx>> {
        let span = source_info.span;
        match *rvalue {
            Rvalue::Use(ref op) => {
                self.eval_operand(op, source_info)
            },
            Rvalue::Repeat(..) |
            Rvalue::Ref(..) |
            Rvalue::Aggregate(..) |
            Rvalue::NullaryOp(NullOp::Box, _) |
            Rvalue::Discriminant(..) => None,

            Rvalue::Cast(kind, ref operand, _) => {
                let op = self.eval_operand(operand, source_info)?;
                self.use_ecx(source_info, |this| {
                    let dest = this.ecx.allocate(place_layout, MemoryKind::Stack);
                    this.ecx.cast(op, kind, dest.into())?;
                    Ok(dest.into())
                })
            }

            // FIXME(oli-obk): evaluate static/constant slice lengths
            Rvalue::Len(_) => None,
            Rvalue::NullaryOp(NullOp::SizeOf, ty) => {
                type_size_of(self.tcx, self.param_env, ty).and_then(|n| Some(
                    ImmTy {
                        imm: Immediate::Scalar(
                            Scalar::Bits {
                                bits: n as u128,
                                size: self.tcx.data_layout.pointer_size.bytes() as u8,
                            }.into()
                        ),
                        layout: self.tcx.layout_of(self.param_env.and(self.tcx.types.usize)).ok()?,
                    }.into()
                ))
            }
            Rvalue::UnaryOp(op, ref arg) => {
                let def_id = if self.tcx.is_closure(self.source.def_id()) {
                    self.tcx.closure_base_def_id(self.source.def_id())
                } else {
                    self.source.def_id()
                };
                let generics = self.tcx.generics_of(def_id);
                if generics.requires_monomorphization(self.tcx) {
                    // FIXME: can't handle code with generics
                    return None;
                }

                let arg = self.eval_operand(arg, source_info)?;
                let val = self.use_ecx(source_info, |this| {
                    let prim = this.ecx.read_immediate(arg)?;
                    match op {
                        UnOp::Neg => {
                            // Need to do overflow check here: For actual CTFE, MIR
                            // generation emits code that does this before calling the op.
                            if prim.to_bits()? == (1 << (prim.layout.size.bits() - 1)) {
                                return err!(OverflowNeg);
                            }
                        }
                        UnOp::Not => {
                            // Cannot overflow
                        }
                    }
                    // Now run the actual operation.
                    this.ecx.unary_op(op, prim)
                })?;
                let res = ImmTy {
                    imm: Immediate::Scalar(val.into()),
                    layout: place_layout,
                };
                Some(res.into())
            }
            Rvalue::CheckedBinaryOp(op, ref left, ref right) |
            Rvalue::BinaryOp(op, ref left, ref right) => {
                trace!("rvalue binop {:?} for {:?} and {:?}", op, left, right);
                let right = self.eval_operand(right, source_info)?;
                let def_id = if self.tcx.is_closure(self.source.def_id()) {
                    self.tcx.closure_base_def_id(self.source.def_id())
                } else {
                    self.source.def_id()
                };
                let generics = self.tcx.generics_of(def_id);
                if generics.requires_monomorphization(self.tcx) {
                    // FIXME: can't handle code with generics
                    return None;
                }

                let r = self.use_ecx(source_info, |this| {
                    this.ecx.read_immediate(right)
                })?;
                if op == BinOp::Shr || op == BinOp::Shl {
                    let left_ty = left.ty(&self.local_decls, self.tcx);
                    let left_bits = self
                        .tcx
                        .layout_of(self.param_env.and(left_ty))
                        .unwrap()
                        .size
                        .bits();
                    let right_size = right.layout.size;
                    let r_bits = r.to_scalar().and_then(|r| r.to_bits(right_size));
                    if r_bits.ok().map_or(false, |b| b >= left_bits as u128) {
                        let source_scope_local_data = match self.source_scope_local_data {
                            ClearCrossCrate::Set(ref data) => data,
                            ClearCrossCrate::Clear => return None,
                        };
                        let dir = if op == BinOp::Shr {
                            "right"
                        } else {
                            "left"
                        };
                        let hir_id = source_scope_local_data[source_info.scope].lint_root;
                        self.tcx.lint_hir(
                            ::rustc::lint::builtin::EXCEEDING_BITSHIFTS,
                            hir_id,
                            span,
                            &format!("attempt to shift {} with overflow", dir));
                        return None;
                    }
                }
                let left = self.eval_operand(left, source_info)?;
                let l = self.use_ecx(source_info, |this| {
                    this.ecx.read_immediate(left)
                })?;
                trace!("const evaluating {:?} for {:?} and {:?}", op, left, right);
                let (val, overflow) = self.use_ecx(source_info, |this| {
                    this.ecx.binary_op(op, l, r)
                })?;
                let val = if let Rvalue::CheckedBinaryOp(..) = *rvalue {
                    Immediate::ScalarPair(
                        val.into(),
                        Scalar::from_bool(overflow).into(),
                    )
                } else {
                    if overflow {
                        let err = InterpError::Overflow(op).into();
                        let _: Option<()> = self.use_ecx(source_info, |_| Err(err));
                        return None;
                    }
                    Immediate::Scalar(val.into())
                };
                let res = ImmTy {
                    imm: val,
                    layout: place_layout,
                };
                Some(res.into())
            },
        }
    }

    fn operand_from_scalar(&self, scalar: Scalar, ty: Ty<'tcx>, span: Span) -> Operand<'tcx> {
        Operand::Constant(Box::new(
            Constant {
                span,
                ty,
                user_ty: None,
                literal: self.tcx.mk_const(ty::Const::from_scalar(
                    scalar,
                    ty,
                ))
            }
        ))
    }

    fn replace_with_const(&self, rval: &mut Rvalue<'tcx>, value: Const<'tcx>, span: Span) {
        self.ecx.validate_operand(
            value,
            vec![],
            None,
            true,
        ).expect("value should already be a valid const");

        if let interpret::Operand::Immediate(im) = *value {
            match im {
                interpret::Immediate::Scalar(ScalarMaybeUndef::Scalar(scalar)) => {
                    *rval = Rvalue::Use(self.operand_from_scalar(scalar, value.layout.ty, span));
                },
                Immediate::ScalarPair(
                    ScalarMaybeUndef::Scalar(one),
                    ScalarMaybeUndef::Scalar(two)
                ) => {
                    let ty = &value.layout.ty.sty;
                    if let ty::Tuple(substs) = ty {
                        *rval = Rvalue::Aggregate(
                            Box::new(AggregateKind::Tuple),
                            vec![
                                self.operand_from_scalar(one, substs[0].expect_ty(), span),
                                self.operand_from_scalar(two, substs[1].expect_ty(), span),
                            ],
                        );
                    }
                },
                _ => { }
            }
        }
    }
}

fn type_size_of<'a, 'tcx>(tcx: TyCtxt<'a, 'tcx, 'tcx>,
                          param_env: ty::ParamEnv<'tcx>,
                          ty: Ty<'tcx>) -> Option<u64> {
    tcx.layout_of(param_env.and(ty)).ok().map(|layout| layout.size.bytes())
}

struct CanConstProp {
    can_const_prop: IndexVec<Local, bool>,
    // false at the beginning, once set, there are not allowed to be any more assignments
    found_assignment: IndexVec<Local, bool>,
}

impl CanConstProp {
    /// returns true if `local` can be propagated
    fn check(mir: &Mir<'_>) -> IndexVec<Local, bool> {
        let mut cpv = CanConstProp {
            can_const_prop: IndexVec::from_elem(true, &mir.local_decls),
            found_assignment: IndexVec::from_elem(false, &mir.local_decls),
        };
        for (local, val) in cpv.can_const_prop.iter_enumerated_mut() {
            // cannot use args at all
            // cannot use locals because if x < y { y - x } else { x - y } would
            //        lint for x != y
            // FIXME(oli-obk): lint variables until they are used in a condition
            // FIXME(oli-obk): lint if return value is constant
            *val = mir.local_kind(local) == LocalKind::Temp;
        }
        cpv.visit_mir(mir);
        cpv.can_const_prop
    }
}

impl<'tcx> Visitor<'tcx> for CanConstProp {
    fn visit_local(
        &mut self,
        &local: &Local,
        context: PlaceContext,
        _: Location,
    ) {
        use rustc::mir::visit::PlaceContext::*;
        match context {
            // Constants must have at most one write
            // FIXME(oli-obk): we could be more powerful here, if the multiple writes
            // only occur in independent execution paths
            MutatingUse(MutatingUseContext::Store) => if self.found_assignment[local] {
                self.can_const_prop[local] = false;
            } else {
                self.found_assignment[local] = true
            },
            // Reading constants is allowed an arbitrary number of times
            NonMutatingUse(NonMutatingUseContext::Copy) |
            NonMutatingUse(NonMutatingUseContext::Move) |
            NonMutatingUse(NonMutatingUseContext::Inspect) |
            NonMutatingUse(NonMutatingUseContext::Projection) |
            MutatingUse(MutatingUseContext::Projection) |
            NonUse(_) => {},
            _ => self.can_const_prop[local] = false,
        }
    }
}

impl<'b, 'a, 'tcx> MutVisitor<'tcx> for ConstPropagator<'b, 'a, 'tcx> {
    fn visit_constant(
        &mut self,
        constant: &mut Constant<'tcx>,
        location: Location,
    ) {
        trace!("visit_constant: {:?}", constant);
        self.super_constant(constant, location);
        self.eval_constant(constant);
    }

    fn visit_statement(
        &mut self,
        statement: &mut Statement<'tcx>,
        location: Location,
    ) {
        trace!("visit_statement: {:?}", statement);
        if let StatementKind::Assign(ref place, ref mut rval) = statement.kind {
            let place_ty: Ty<'tcx> = place
                .ty(&self.local_decls, self.tcx)
                .ty;
            if let Ok(place_layout) = self.tcx.layout_of(self.param_env.and(place_ty)) {
                if let Some(value) = self.const_prop(rval, place_layout, statement.source_info) {
                    if let Place::Base(PlaceBase::Local(local)) = *place {
                        trace!("checking whether {:?} can be stored to {:?}", value, local);
                        if self.can_const_prop[local] {
                            trace!("storing {:?} to {:?}", value, local);
                            assert!(self.places[local].is_none());
                            self.places[local] = Some(value);

                            if self.tcx.sess.opts.debugging_opts.mir_opt_level >= 3 {
                                self.replace_with_const(rval, value, statement.source_info.span);
                            }
                        }
                    }
                }
            }
        }
        self.super_statement(statement, location);
    }

    fn visit_terminator(
        &mut self,
        terminator: &mut Terminator<'tcx>,
        location: Location,
    ) {
        self.super_terminator(terminator, location);
        let source_info = terminator.source_info;;
        if let TerminatorKind::Assert { expected, msg, cond, .. } = &terminator.kind {
            if let Some(value) = self.eval_operand(&cond, source_info) {
                trace!("assertion on {:?} should be {:?}", value, expected);
                let expected = ScalarMaybeUndef::from(Scalar::from_bool(*expected));
                if expected != self.ecx.read_scalar(value).unwrap() {
                    // poison all places this operand references so that further code
                    // doesn't use the invalid value
                    match cond {
                        Operand::Move(ref place) | Operand::Copy(ref place) => {
                            let mut place = place;
                            while let Place::Projection(ref proj) = *place {
                                place = &proj.base;
                            }
                            if let Place::Base(PlaceBase::Local(local)) = *place {
                                self.places[local] = None;
                            }
                        },
                        Operand::Constant(_) => {}
                    }
                    let span = terminator.source_info.span;
                    let hir_id = self
                        .tcx
                        .hir()
                        .as_local_hir_id(self.source.def_id())
                        .expect("some part of a failing const eval must be local");
                    use rustc::mir::interpret::InterpError::*;
                    let msg = match msg {
                        Overflow(_) |
                        OverflowNeg |
                        DivisionByZero |
                        RemainderByZero => msg.description().to_owned(),
                        BoundsCheck { ref len, ref index } => {
                            let len = self
                                .eval_operand(len, source_info)
                                .expect("len must be const");
                            let len = match self.ecx.read_scalar(len) {
                                Ok(ScalarMaybeUndef::Scalar(Scalar::Bits {
                                    bits, ..
                                })) => bits,
                                other => bug!("const len not primitive: {:?}", other),
                            };
                            let index = self
                                .eval_operand(index, source_info)
                                .expect("index must be const");
                            let index = match self.ecx.read_scalar(index) {
                                Ok(ScalarMaybeUndef::Scalar(Scalar::Bits {
                                    bits, ..
                                })) => bits,
                                other => bug!("const index not primitive: {:?}", other),
                            };
                            format!(
                                "index out of bounds: \
                                the len is {} but the index is {}",
                                len,
                                index,
                            )
                        },
                        // Need proper const propagator for these
                        _ => return,
                    };
                    self.tcx.lint_hir(
                        ::rustc::lint::builtin::CONST_ERR,
                        hir_id,
                        span,
                        &msg,
                    );
                }
            }
        }
    }
}
