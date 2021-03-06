import front.ast;
import front.ast.ident;
import front.ast.def;
import front.ast.ann;
import front.creader;
import driver.session;
import util.common.new_def_hash;
import util.common.span;
import util.typestate_ann.ts_ann;
import std.map.hashmap;
import std.list.list;
import std.list.nil;
import std.list.cons;
import std.option;
import std.option.some;
import std.option.none;
import std._str;
import std._vec;

tag scope {
    scope_crate(@ast.crate);
    scope_item(@ast.item);
    scope_native_item(@ast.native_item);
    scope_external_mod(ast.def_id, vec[ast.ident]);
    scope_loop(@ast.decl); // there's only 1 decl per loop.
    scope_block(ast.block);
    scope_arm(ast.arm);
}

type env = rec(list[scope] scopes,
               session.session sess);

tag namespace {
    ns_value;
    ns_type;
}

type import_map = std.map.hashmap[ast.def_id,def_wrap];

// A simple wrapper over defs that stores a bit more information about modules
// and uses so that we can use the regular lookup_name when resolving imports.
tag def_wrap {
    def_wrap_use(@ast.view_item);
    def_wrap_import(@ast.view_item);
    def_wrap_mod(@ast.item);
    def_wrap_native_mod(@ast.item);
    def_wrap_external_mod(ast.def_id);
    def_wrap_external_native_mod(ast.def_id);
    def_wrap_other(def);
    def_wrap_expr_field(uint, def);
    def_wrap_resolving;
}

fn unwrap_def(def_wrap d) -> def {
    alt (d) {
        case (def_wrap_use(?it)) {
            alt (it.node) {
                case (ast.view_item_use(_, _, ?id, _)) {
                    ret ast.def_use(id);
                }
            }
        }
        case (def_wrap_import(?it)) {
            alt (it.node) {
                case (ast.view_item_import(_, _, ?id, ?target_def)) {
                    alt (target_def) {
                        case (some[def](?d)) {
                            ret d;
                        }
                        case (none[def]) {
                            fail;
                        }
                    }
                }
            }
        }
        case (def_wrap_mod(?m)) {
            alt (m.node) {
                case (ast.item_mod(_, _, ?id)) {
                    ret ast.def_mod(id);
                }
            }
        }
        case (def_wrap_native_mod(?m)) {
            alt (m.node) {
                case (ast.item_native_mod(_, _, ?id)) {
                    ret ast.def_native_mod(id);
                }
            }
        }
        case (def_wrap_external_mod(?mod_id)) {
            ret ast.def_mod(mod_id);
        }
        case (def_wrap_external_native_mod(?mod_id)) {
            ret ast.def_native_mod(mod_id);
        }
        case (def_wrap_other(?d)) {
            ret d;
        }
        case (def_wrap_expr_field(_, ?d)) {
            ret d;
        }
    }
}

fn lookup_external_def(session.session sess, int cnum, vec[ident] idents)
        -> option.t[def_wrap] {
    alt (creader.lookup_def(sess, cnum, idents)) {
        case (none[ast.def]) {
            ret none[def_wrap];
        }
        case (some[ast.def](?def)) {
            auto dw;
            alt (def) {
                case (ast.def_mod(?mod_id)) {
                    dw = def_wrap_external_mod(mod_id);
                }
                case (ast.def_native_mod(?mod_id)) {
                    dw = def_wrap_external_native_mod(mod_id);
                }
                case (_) {
                    dw = def_wrap_other(def);
                }
            }
            ret some[def_wrap](dw);
        }
    }
}

// Follow the path of an import and return what it ultimately points to.

// If used after imports are resolved, import_id is none.

fn find_final_def(&env e, import_map index,
                  &span sp, vec[ident] idents, namespace ns,
                  option.t[ast.def_id] import_id) -> def_wrap {

    // We are given a series of identifiers (p.q.r) and we know that
    // in the environment 'e' the identifier 'p' was resolved to 'd'. We
    // should return what p.q.r points to in the end.
    fn found_something(&env e, import_map index,
                       &span sp, vec[ident] idents, namespace ns,
                       def_wrap d) -> def_wrap {

        fn found_mod(&env e, &import_map index, &span sp,
                     vec[ident] idents, namespace ns,
                     @ast.item i) -> def_wrap {
            auto len = _vec.len[ident](idents);
            auto rest_idents = _vec.slice[ident](idents, 1u, len);
            auto empty_e = rec(scopes = nil[scope],
                               sess = e.sess);
            auto tmp_e = update_env_for_item(empty_e, i);
            auto next_i = rest_idents.(0);
            auto next_ = lookup_name_wrapped(tmp_e, next_i, ns);
            alt (next_) {
                case (none[tup(@env, def_wrap)]) {
                    e.sess.span_err(sp, "unresolved name: " + next_i);
                    fail;
                }
                case (some[tup(@env, def_wrap)](?next)) {
                    auto combined_e = update_env_for_item(e, i);
                    ret found_something(combined_e, index, sp,
                                        rest_idents, ns, next._1);
                }
            }
        }

        // TODO: Refactor with above.
        fn found_external_mod(&env e, &import_map index, &span sp,
                              vec[ident] idents, namespace ns,
                              ast.def_id mod_id)
                -> def_wrap {
            auto len = _vec.len[ident](idents);
            auto rest_idents = _vec.slice[ident](idents, 1u, len);
            auto empty_e = rec(scopes = nil[scope], sess = e.sess);
            auto tmp_e = update_env_for_external_mod(empty_e, mod_id, idents);
            auto next_i = rest_idents.(0);
            auto next_ = lookup_name_wrapped(tmp_e, next_i, ns);
            alt (next_) {
                case (none[tup(@env, def_wrap)]) {
                    e.sess.span_err(sp, "unresolved name: " + next_i);
                    fail;
                }
                case (some[tup(@env, def_wrap)](?next)) {
                    auto combined_e = update_env_for_external_mod(e,
                                                                  mod_id,
                                                                  idents);
                    ret found_something(combined_e, index, sp,
                                        rest_idents, ns, next._1);
                }
            }
        }

        fn found_crate(&env e, &import_map index, &span sp,
                       vec[ident] idents, int cnum) -> def_wrap {
            auto len = _vec.len[ident](idents);
            auto rest_idents = _vec.slice[ident](idents, 1u, len);
            alt (lookup_external_def(e.sess, cnum, rest_idents)) {
                case (none[def_wrap]) {
                    e.sess.span_err(sp, #fmt("unbound name '%s'",
                                             _str.connect(idents, ".")));
                    fail;
                }
                case (some[def_wrap](?dw)) { ret dw; }
            }
        }

        alt (d) {
            case (def_wrap_import(?imp)) {
                alt (imp.node) {
                    case (ast.view_item_import(_, ?new_idents, ?d, _)) {
                        auto x = find_final_def(e, index, sp, new_idents, ns,
                                                some(d));
                        ret found_something(e, index, sp, idents, ns, x);
                    }
                }
            }
            case (_) {
            }
        }
        auto len = _vec.len[ident](idents);
        if (len == 1u) {
            ret d;
        }
        alt (d) {
            case (def_wrap_mod(?i)) {
                ret found_mod(e, index, sp, idents, ns, i);
            }
            case (def_wrap_native_mod(?i)) {
                ret found_mod(e, index, sp, idents, ns, i);
            }
            case (def_wrap_external_mod(?mod_id)) {
                ret found_external_mod(e, index, sp, idents, ns, mod_id);
            }
            case (def_wrap_external_native_mod(?mod_id)) {
                ret found_external_mod(e, index, sp, idents, ns, mod_id);
            }
            case (def_wrap_use(?vi)) {
                alt (vi.node) {
                    case (ast.view_item_use(_, _, _, ?cnum_opt)) {
                        auto cnum = option.get[int](cnum_opt);
                        ret found_crate(e, index, sp, idents, cnum);
                    }
                }
            }
            case (def_wrap_other(?d)) {
                let uint l = _vec.len[ident](idents);
                ret def_wrap_expr_field(l, d);
            }
        }
        fail;
    }

    if (import_id != none[ast.def_id]) {
        alt (index.find(option.get[ast.def_id](import_id))) {
            case (some[def_wrap](?x)) {
                alt (x) {
                    case (def_wrap_resolving) {
                        e.sess.span_err(sp, "cyclic import");
                        fail;
                    }
                    case (_) {
                        ret x;
                    }
                }
            }
            case (none[def_wrap]) {
            }
        }
        index.insert(option.get[ast.def_id](import_id), def_wrap_resolving);
    }
    auto first = idents.(0);
    auto d_ = lookup_name_wrapped(e, first, ns);
    alt (d_) {
        case (none[tup(@env, def_wrap)]) {
            e.sess.span_err(sp, "unresolved name: " + first);
            fail;
        }
        case (some[tup(@env, def_wrap)](?d)) {
            auto x = found_something(*d._0, index, sp, idents, ns, d._1);
            if (import_id != none[ast.def_id]) {
                index.insert(option.get[ast.def_id](import_id), x);
            }
            ret x;
        }
    }
}

fn lookup_name_wrapped(&env e, ast.ident i, namespace ns)
        -> option.t[tup(@env, def_wrap)] {

    // log "resolving name " + i;

    fn found_def_item(@ast.item i, namespace ns) -> def_wrap {
        alt (i.node) {
            case (ast.item_const(_, _, _, ?id, _)) {
                ret def_wrap_other(ast.def_const(id));
            }
            case (ast.item_fn(_, _, _, ?id, _)) {
                ret def_wrap_other(ast.def_fn(id));
            }
            case (ast.item_mod(_, _, ?id)) {
                ret def_wrap_mod(i);
            }
            case (ast.item_native_mod(_, _, ?id)) {
                ret def_wrap_native_mod(i);
            }
            case (ast.item_ty(_, _, _, ?id, _)) {
                ret def_wrap_other(ast.def_ty(id));
            }
            case (ast.item_tag(_, _, _, ?id, _)) {
                ret def_wrap_other(ast.def_ty(id));
            }
            case (ast.item_obj(_, _, _, ?odid, _)) {
                alt (ns) {
                    case (ns_value) {
                        ret def_wrap_other(ast.def_obj(odid.ctor));
                    }
                    case (ns_type) {
                        ret def_wrap_other(ast.def_obj(odid.ty));
                    }
                }
            }
        }
    }

    fn found_def_native_item(@ast.native_item i) -> def_wrap {
        alt (i.node) {
            case (ast.native_item_ty(_, ?id)) {
                ret def_wrap_other(ast.def_native_ty(id));
            }
            case (ast.native_item_fn(_, _, _, _, ?id, _)) {
                ret def_wrap_other(ast.def_native_fn(id));
            }
        }
    }

    fn found_def_view(@ast.view_item i) -> def_wrap {
        alt (i.node) {
            case (ast.view_item_use(_, _, ?id, _)) {
                ret def_wrap_use(i);
            }
            case (ast.view_item_import(_, ?idents,?d, _)) {
                ret def_wrap_import(i);
            }
        }
        fail;
    }

    fn check_mod(ast.ident i, ast._mod m, namespace ns)
            -> option.t[def_wrap] {
        alt (m.index.find(i)) {
            case (some[ast.mod_index_entry](?ent)) {
                alt (ent) {
                    case (ast.mie_view_item(?view_item)) {
                        ret some(found_def_view(view_item));
                    }
                    case (ast.mie_item(?item)) {
                        ret some(found_def_item(item, ns));
                    }
                    case (ast.mie_tag_variant(?item, ?variant_idx)) {
                        alt (item.node) {
                            case (ast.item_tag(_, ?variants, _, ?tid, _)) {
                                auto vid = variants.(variant_idx).node.id;
                                auto t = ast.def_variant(tid, vid);
                                ret some[def_wrap](def_wrap_other(t));
                            }
                            case (_) {
                                log_err "tag item not actually a tag";
                                fail;
                            }
                        }
                    }
                }
            }
            case (none[ast.mod_index_entry]) {
                ret none[def_wrap];
            }
        }
    }

    fn check_native_mod(ast.ident i, ast.native_mod m) -> option.t[def_wrap] {

        alt (m.index.find(i)) {
            case (some[ast.native_mod_index_entry](?ent)) {
                alt (ent) {
                    case (ast.nmie_view_item(?view_item)) {
                        ret some(found_def_view(view_item));
                    }
                    case (ast.nmie_item(?item)) {
                        ret some(found_def_native_item(item));
                    }
                }
            }
            case (none[ast.native_mod_index_entry]) {
                ret none[def_wrap];
            }
        }
    }

    fn handle_fn_decl(ast.ident identifier, &ast.fn_decl decl,
                      &vec[ast.ty_param] ty_params) -> option.t[def_wrap] {
        for (ast.arg a in decl.inputs) {
            if (_str.eq(a.ident, identifier)) {
                auto t = ast.def_arg(a.id);
                ret some(def_wrap_other(t));
            }
        }

        auto i = 0u;
        for (ast.ty_param tp in ty_params) {
            if (_str.eq(tp, identifier)) {
                auto t = ast.def_ty_arg(i);
                ret some(def_wrap_other(t));
            }
            i += 1u;
        }
        ret none[def_wrap];
    }

    fn found_tag(@ast.item item, uint variant_idx) -> def_wrap {
        alt (item.node) {
            case (ast.item_tag(_, ?variants, _, ?tid, _)) {
                auto vid = variants.(variant_idx).node.id;
                auto t = ast.def_variant(tid, vid);
                ret def_wrap_other(t);
            }
            case (_) {
                log_err "tag item not actually a tag";
                fail;
            }
        }
    }

    fn check_block(ast.ident i, &ast.block_ b, namespace ns)
            -> option.t[def_wrap] {
        alt (b.index.find(i)) {
            case (some[ast.block_index_entry](?ix)) {
                alt(ix) {
                    case (ast.bie_item(?it)) {
                        ret some(found_def_item(it, ns));
                    }
                    case (ast.bie_local(?l)) {
                        auto t = ast.def_local(l.id);
                        ret some(def_wrap_other(t));
                    }
                    case (ast.bie_tag_variant(?item, ?variant_idx)) {
                        ret some(found_tag(item, variant_idx));
                    }
                }
            }
            case (_) { ret none[def_wrap]; }
        }
    }

    fn in_scope(&session.session sess, ast.ident identifier, &scope s,
            namespace ns) -> option.t[def_wrap] {
        alt (s) {

            case (scope_crate(?c)) {
                ret check_mod(identifier, c.node.module, ns);
            }

            case (scope_item(?it)) {
                alt (it.node) {
                    case (ast.item_fn(_, ?f, ?ty_params, _, _)) {
                        ret handle_fn_decl(identifier, f.decl, ty_params);
                    }
                    case (ast.item_obj(_, ?ob, ?ty_params, _, _)) {
                        for (ast.obj_field f in ob.fields) {
                            if (_str.eq(f.ident, identifier)) {
                                auto t = ast.def_obj_field(f.id);
                                ret some(def_wrap_other(t));
                            }
                        }

                        auto i = 0u;
                        for (ast.ty_param tp in ty_params) {
                            if (_str.eq(tp, identifier)) {
                                auto t = ast.def_ty_arg(i);
                                ret some(def_wrap_other(t));
                            }
                            i += 1u;
                        }
                    }
                    case (ast.item_tag(_,?variants,?ty_params,?tag_id,_)) {
                        auto i = 0u;
                        for (ast.ty_param tp in ty_params) {
                            if (_str.eq(tp, identifier)) {
                                auto t = ast.def_ty_arg(i);
                                ret some(def_wrap_other(t));
                            }
                            i += 1u;
                        }
                    }
                    case (ast.item_mod(_, ?m, _)) {
                        ret check_mod(identifier, m, ns);
                    }
                    case (ast.item_native_mod(_, ?m, _)) {
                        ret check_native_mod(identifier, m);
                    }
                    case (ast.item_ty(_, _, ?ty_params, _, _)) {
                        auto i = 0u;
                        for (ast.ty_param tp in ty_params) {
                            if (_str.eq(tp, identifier)) {
                                auto t = ast.def_ty_arg(i);
                                ret some(def_wrap_other(t));
                            }
                            i += 1u;
                        }
                    }
                    case (_) { /* fall through */ }
                }
            }

            case (scope_native_item(?it)) {
                alt (it.node) {
                    case (ast.native_item_fn(_, _, ?decl, ?ty_params, _, _)) {
                        ret handle_fn_decl(identifier, decl, ty_params);
                    }
                }
            }

            case (scope_external_mod(?mod_id, ?path)) {
                ret lookup_external_def(sess, mod_id._0, path);
            }

            case (scope_loop(?d)) {
                alt (d.node) {
                    case (ast.decl_local(?local)) {
                        if (_str.eq(local.ident, identifier)) {
                            auto lc = ast.def_local(local.id);
                            ret some(def_wrap_other(lc));
                        }
                    }
                }
            }

            case (scope_block(?b)) {
                ret check_block(identifier, b.node, ns);
            }

            case (scope_arm(?a)) {
                alt (a.index.find(identifier)) {
                    case (some[ast.def_id](?did)) {
                        auto t = ast.def_binding(did);
                        ret some[def_wrap](def_wrap_other(t));
                    }
                    case (_) { /* fall through */  }
                }
            }
        }
        ret none[def_wrap];
    }

    alt (e.scopes) {
        case (nil[scope]) {
            ret none[tup(@env, def_wrap)];
        }
        case (cons[scope](?hd, ?tl)) {
            auto x = in_scope(e.sess, i, hd, ns);
            alt (x) {
                case (some[def_wrap](?x)) {
                    ret some(tup(@e, x));
                }
                case (none[def_wrap]) {
                    auto outer_env = rec(scopes = *tl with e);
                    ret lookup_name_wrapped(outer_env, i, ns);
                }
            }
        }
    }
}

fn fold_pat_tag(&env e, &span sp, ast.path p, vec[@ast.pat] args,
                option.t[ast.variant_def] old_def,
                ann a) -> @ast.pat {
    auto len = _vec.len[ast.ident](p.node.idents);
    auto last_id = p.node.idents.(len - 1u);
    auto new_def;
    auto index = new_def_hash[def_wrap]();
    auto d = find_final_def(e, index, sp, p.node.idents, ns_value,
                            none[ast.def_id]);
    alt (unwrap_def(d)) {
        case (ast.def_variant(?did, ?vid)) {
            new_def = some[ast.variant_def](tup(did, vid));
        }
        case (_) {
            e.sess.span_err(sp, "not a tag variant: " + last_id);
            new_def = none[ast.variant_def];
        }
    }

    ret @fold.respan[ast.pat_](sp, ast.pat_tag(p, args, new_def, a));
}

// We received a path expression of the following form:
//
//     a.b.c.d
//
// Somewhere along this path there might be a split from a path-expr
// to a runtime field-expr. For example:
//
//     'a' could be the name of a variable in the local scope
//     and 'b.c.d' could be a field-sequence inside it.
//
// Or:
//
//     'a.b' could be a module path to a constant record, and 'c.d'
//     could be a field within it.
//
// Our job here is to figure out what the prefix of 'a.b.c.d' is that
// corresponds to a static binding-name (a module or slot, with no type info)
// and split that off as the 'primary' expr_path, with secondary expr_field
// expressions tacked on the end.

fn fold_expr_path(&env e, &span sp, &ast.path p, &option.t[def] d,
                  ann a) -> @ast.expr {
    auto n_idents = _vec.len[ast.ident](p.node.idents);
    check (n_idents != 0u);

    auto index = new_def_hash[def_wrap]();
    auto d = find_final_def(e, index, sp, p.node.idents, ns_value,
                            none[ast.def_id]);
    let uint path_len = 0u;
    alt (d) {
        case (def_wrap_expr_field(?remaining, _)) {
            path_len = n_idents - remaining + 1u;
        }
        case (def_wrap_other(_)) {
            path_len = n_idents;
        }
        case (def_wrap_mod(?m)) {
            e.sess.span_err(sp,
                            "can't refer to a module as a first-class value");
            fail;
        }
    }
    auto path_elems =
        _vec.slice[ident](p.node.idents, 0u, path_len);
    auto p_ = rec(node=rec(idents = path_elems with p.node) with p);
    auto d_ = some(unwrap_def(d));
    auto ex = @fold.respan[ast.expr_](sp, ast.expr_path(p_, d_, a));
    auto i = path_len;
    while (i < n_idents) {
        auto id = p.node.idents.(i);
        ex = @fold.respan[ast.expr_](sp, ast.expr_field(ex, id, a));
        i += 1u;
    }
    ret ex;
}

fn fold_view_item_import(&env e, &span sp,
                         import_map index, ident i,
                         vec[ident] is, ast.def_id id,
                         option.t[def] target_id) -> @ast.view_item {
    // Produce errors for invalid imports
    auto len = _vec.len[ast.ident](is);
    auto last_id = is.(len - 1u);
    auto d = find_final_def(e, index, sp, is, ns_value, some(id));
    alt (d) {
        case (def_wrap_expr_field(?remain, _)) {
            auto ident = is.(len - remain);
            e.sess.span_err(sp, ident + " is not a module or crate");
        }
        case (_) {
        }
    }
    let option.t[def] target_def = some(unwrap_def(d));
    ret @fold.respan[ast.view_item_](sp, ast.view_item_import(i, is, id,
                                                              target_def));
}

fn fold_ty_path(&env e, &span sp, ast.path p, &option.t[def] d) -> @ast.ty {
    auto index = new_def_hash[def_wrap]();
    auto d = find_final_def(e, index, sp, p.node.idents, ns_type,
                            none[ast.def_id]);

    ret @fold.respan[ast.ty_](sp, ast.ty_path(p, some(unwrap_def(d))));
}

fn update_env_for_crate(&env e, @ast.crate c) -> env {
    ret rec(scopes = cons[scope](scope_crate(c), @e.scopes) with e);
}

fn update_env_for_item(&env e, @ast.item i) -> env {
    ret rec(scopes = cons[scope](scope_item(i), @e.scopes) with e);
}

fn update_env_for_native_item(&env e, @ast.native_item i) -> env {
    ret rec(scopes = cons[scope](scope_native_item(i), @e.scopes) with e);
}

// Not actually called by fold, but here since this is analogous to
// update_env_for_item() above and is called by find_final_def().
fn update_env_for_external_mod(&env e, ast.def_id mod_id,
                               vec[ast.ident] idents) -> env {
    ret rec(scopes = cons[scope](scope_external_mod(mod_id, idents),
                                 @e.scopes)
            with e);
}

fn update_env_for_block(&env e, &ast.block b) -> env {
    ret rec(scopes = cons[scope](scope_block(b), @e.scopes) with e);
}

fn update_env_for_expr(&env e, @ast.expr x) -> env {
    alt (x.node) {
        case (ast.expr_for(?d, _, _, _)) {
            ret rec(scopes = cons[scope](scope_loop(d), @e.scopes) with e);
        }
        case (ast.expr_for_each(?d, _, _, _)) {
            ret rec(scopes = cons[scope](scope_loop(d), @e.scopes) with e);
        }
        case (_) { }
    }
    ret e;
}

fn update_env_for_arm(&env e, &ast.arm p) -> env {
    ret rec(scopes = cons[scope](scope_arm(p), @e.scopes) with e);
}

fn resolve_imports(session.session sess, @ast.crate crate) -> @ast.crate {
    let fold.ast_fold[env] fld = fold.new_identity_fold[env]();

    auto import_index = new_def_hash[def_wrap]();
    fld = @rec( fold_view_item_import
                    = bind fold_view_item_import(_,_,import_index,_,_,_,_),
                update_env_for_crate = bind update_env_for_crate(_,_),
                update_env_for_item = bind update_env_for_item(_,_),
                update_env_for_native_item =
                    bind update_env_for_native_item(_,_),
                update_env_for_block = bind update_env_for_block(_,_),
                update_env_for_arm = bind update_env_for_arm(_,_),
                update_env_for_expr = bind update_env_for_expr(_,_)
                with *fld );

    auto e = rec(scopes = nil[scope],
                 sess = sess);

    ret fold.fold_crate[env](e, fld, crate);
}

fn resolve_crate(session.session sess, @ast.crate crate) -> @ast.crate {

    let fold.ast_fold[env] fld = fold.new_identity_fold[env]();

    auto new_crate = resolve_imports(sess, crate);

    fld = @rec( fold_pat_tag = bind fold_pat_tag(_,_,_,_,_,_),
                fold_expr_path = bind fold_expr_path(_,_,_,_,_),
                fold_ty_path = bind fold_ty_path(_,_,_,_),
                update_env_for_crate = bind update_env_for_crate(_,_),
                update_env_for_item = bind update_env_for_item(_,_),
                update_env_for_native_item =
                    bind update_env_for_native_item(_,_),
                update_env_for_block = bind update_env_for_block(_,_),
                update_env_for_arm = bind update_env_for_arm(_,_),
                update_env_for_expr = bind update_env_for_expr(_,_)
                with *fld );

    auto e = rec(scopes = nil[scope],
                 sess = sess);

    ret fold.fold_crate[env](e, fld, new_crate);
}

// Local Variables:
// mode: rust
// fill-column: 78;
// indent-tabs-mode: nil
// c-basic-offset: 4
// buffer-file-coding-system: utf-8-unix
// compile-command: "make -k -C $RBUILD 2>&1 | sed -e 's/\\/x\\//x:\\//g'";
// End:
