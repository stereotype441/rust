// -*- rust -*-

import driver.session;
import front.ast;
import lib.llvm.False;
import lib.llvm.llvm;
import lib.llvm.mk_object_file;
import lib.llvm.mk_section_iter;
import middle.fold;
import middle.metadata;
import middle.trans;
import middle.ty;
import back.x86;
import util.common;
import util.common.span;

import std._str;
import std._uint;
import std._vec;
import std.ebml;
import std.fs;
import std.io;
import std.option;
import std.option.none;
import std.option.some;
import std.os;
import std.map.hashmap;

// TODO: map to a real type here.
type env = @rec(
    session.session sess,
    @hashmap[str, int] crate_cache,
    vec[str] library_search_paths,
    mutable int next_crate_num
);

tag resolve_result {
    rr_ok(ast.def_id);
    rr_not_found(ast.ident);
}

// Type decoding

// Compact string representation for ty.t values. API ty_str & parse_from_str.
// (The second has to be authed pure.) Extra parameters are for converting
// to/from def_ids in the string rep. Whatever format you choose should not
// contain pipe characters.

// Callback to translate defs to strs or back.
type str_def = fn(str) -> ast.def_id;

type pstate = rec(str rep, mutable uint pos, uint len,
                  @ty.type_store tystore);

fn peek(@pstate st) -> u8 {
    if (st.pos < st.len) {ret st.rep.(st.pos) as u8;}
    else {ret ' ' as u8;}
}
fn next(@pstate st) -> u8 {
    if (st.pos >= st.len) {fail;}
    auto ch = st.rep.(st.pos);
    st.pos = st.pos + 1u;
    ret ch as u8;
}

fn parse_ty_str(str rep, str_def sd, @ty.type_store tystore) -> @ty.t {
    auto len = _str.byte_len(rep);
    auto st = @rec(rep=rep, mutable pos=0u, len=len, tystore=tystore);
    auto result = parse_ty(st, sd);
    if (st.pos != len) {
        log_err "parse_ty_str: incomplete parse, stopped at byte "
            + _uint.to_str(st.pos, 10u) + " of "
            + _uint.to_str(len, 10u) + " in str '" + rep + "'";
    }
    ret result;
}

fn parse_ty(@pstate st, str_def sd) -> @ty.t {
    alt (next(st) as char) {
        case ('n') { ret ty.mk_nil(st.tystore); }
        case ('b') { ret ty.mk_bool(st.tystore); }
        case ('i') { ret ty.mk_int(st.tystore); }
        case ('u') { ret ty.mk_uint(st.tystore); }
        case ('l') { ret ty.mk_float(st.tystore); }
        case ('M') {
            alt (next(st) as char) {
                case ('b') { ret ty.mk_mach(st.tystore, common.ty_u8); }
                case ('w') { ret ty.mk_mach(st.tystore, common.ty_u16); }
                case ('l') { ret ty.mk_mach(st.tystore, common.ty_u32); }
                case ('d') { ret ty.mk_mach(st.tystore, common.ty_u64); }
                case ('B') { ret ty.mk_mach(st.tystore, common.ty_i8); }
                case ('W') { ret ty.mk_mach(st.tystore, common.ty_i16); }
                case ('L') { ret ty.mk_mach(st.tystore, common.ty_i32); }
                case ('D') { ret ty.mk_mach(st.tystore, common.ty_i64); }
                case ('f') { ret ty.mk_mach(st.tystore, common.ty_f32); }
                case ('F') { ret ty.mk_mach(st.tystore, common.ty_f64); }
            }
        }
        case ('c') { ret ty.mk_char(st.tystore); }
        case ('s') { ret ty.mk_str(st.tystore); }
        case ('t') {
            check(next(st) as char == '[');
            auto def = parse_def(st, sd);
            let vec[@ty.t] params = vec();
            while (peek(st) as char != ']') {
                params += vec(parse_ty(st, sd));
            }
            st.pos = st.pos + 1u;
            ret ty.mk_tag(st.tystore, def, params);
        }
        case ('p') { ret ty.mk_param(st.tystore, parse_int(st) as uint); }
        case ('@') { ret ty.mk_box(st.tystore, parse_mt(st, sd)); }
        case ('V') { ret ty.mk_vec(st.tystore, parse_mt(st, sd)); }
        case ('P') { ret ty.mk_port(st.tystore, parse_ty(st, sd)); }
        case ('C') { ret ty.mk_chan(st.tystore, parse_ty(st, sd)); }
        case ('T') {
            check(next(st) as char == '[');
            let vec[ty.mt] params = vec();
            while (peek(st) as char != ']') {
                params += vec(parse_mt(st, sd));
            }
            st.pos = st.pos + 1u;
            ret ty.mk_tup(st.tystore, params);
        }
        case ('R') {
            check(next(st) as char == '[');
            let vec[ty.field] fields = vec();
            while (peek(st) as char != ']') {
                auto name = "";
                while (peek(st) as char != '=') {
                    name += _str.unsafe_from_byte(next(st));
                }
                st.pos = st.pos + 1u;
                fields += vec(rec(ident=name, mt=parse_mt(st, sd)));
            }
            st.pos = st.pos + 1u;
            ret ty.mk_rec(st.tystore, fields);
        }
        case ('F') {
            auto func = parse_ty_fn(st, sd);
            ret ty.mk_fn(st.tystore, ast.proto_fn, func._0, func._1);
        }
        case ('W') {
            auto func = parse_ty_fn(st, sd);
            ret ty.mk_fn(st.tystore, ast.proto_iter, func._0, func._1);
        }
        case ('N') {
            auto abi;
            alt (next(st) as char) {
                case ('r') {abi = ast.native_abi_rust;}
                case ('c') {abi = ast.native_abi_cdecl;}
                case ('l') {abi = ast.native_abi_llvm;}
            }
            auto func = parse_ty_fn(st, sd);
            ret ty.mk_native_fn(st.tystore,abi,func._0,func._1);
        }
        case ('O') {
            check(next(st) as char == '[');
            let vec[ty.method] methods = vec();
            while (peek(st) as char != ']') {
                auto proto;
                alt (next(st) as char) {
                    case ('W') {proto = ast.proto_iter;}
                    case ('F') {proto = ast.proto_fn;}
                }
                auto name = "";
                while (peek(st) as char != '[') {
                    name += _str.unsafe_from_byte(next(st));
                }
                auto func = parse_ty_fn(st, sd);
                methods += vec(rec(proto=proto,
                                   ident=name,
                                   inputs=func._0,
                                   output=func._1));
            }
            st.pos += 1u;
            ret ty.mk_obj(st.tystore, methods);
        }
        case ('X') { ret ty.mk_var(st.tystore, parse_int(st)); }
        case ('E') { ret ty.mk_native(st.tystore); }
        case ('Y') { ret ty.mk_type(st.tystore); }
    }
}

fn parse_mt(@pstate st, str_def sd) -> ty.mt {
    auto mut;
    alt (peek(st) as char) {
        case ('m') {next(st); mut = ast.mut;}
        case ('?') {next(st); mut = ast.maybe_mut;}
        case (_)   {mut=ast.imm;}
    }
    ret rec(ty=parse_ty(st, sd), mut=mut);
}

fn parse_def(@pstate st, str_def sd) -> ast.def_id {
    auto def = "";
    while (peek(st) as char != '|') {
        def += _str.unsafe_from_byte(next(st));
    }
    st.pos = st.pos + 1u;
    ret sd(def);
}

fn parse_int(@pstate st) -> int {
    auto n = 0;
    while (true) {
        auto cur = peek(st) as char;
        if (cur < '0' || cur > '9') {break;}
        st.pos = st.pos + 1u;
        n *= 10;
        n += (cur as int) - ('0' as int);
    }
    ret n;
}

fn parse_ty_fn(@pstate st, str_def sd) -> tup(vec[ty.arg], @ty.t) {
    check(next(st) as char == '[');
    let vec[ty.arg] inputs = vec();
    while (peek(st) as char != ']') {
        auto mode = ast.val;
        if (peek(st) as char == '&') {
            mode = ast.alias;
            st.pos = st.pos + 1u;
        }
        inputs += vec(rec(mode=mode, ty=parse_ty(st, sd)));
    }
    st.pos = st.pos + 1u;
    ret tup(inputs, parse_ty(st, sd));
}


// Rust metadata parsing

fn parse_def_id(vec[u8] buf) -> ast.def_id {
    auto colon_idx = 0u;
    auto len = _vec.len[u8](buf);
    while (colon_idx < len && buf.(colon_idx) != (':' as u8)) {
        colon_idx += 1u;
    }
    if (colon_idx == len) {
        log_err "didn't find ':' when parsing def id";
        fail;
    }

    auto crate_part = _vec.slice[u8](buf, 0u, colon_idx);
    auto def_part = _vec.slice[u8](buf, colon_idx + 1u, len);
    auto crate_num = _uint.parse_buf(crate_part, 10u) as int;
    auto def_num = _uint.parse_buf(def_part, 10u) as int;
    ret tup(crate_num, def_num);
}

fn lookup_hash(&ebml.doc d, fn(vec[u8]) -> bool eq_fn, uint hash)
    -> option.t[ebml.doc] {
    auto index = ebml.get_doc(d, metadata.tag_index);
    auto table = ebml.get_doc(index, metadata.tag_index_table);

    auto hash_pos = table.start + (hash % 256u) * 4u;
    auto pos = ebml.be_uint_from_bytes(d.data, hash_pos, 4u);
    auto bucket = ebml.doc_at(d.data, pos);
    // Awkward logic because we can't ret from foreach yet
    auto result = option.none[ebml.doc];
    auto belt = metadata.tag_index_buckets_bucket_elt;
    for each (ebml.doc elt in ebml.tagged_docs(bucket, belt)) {
        alt (result) {
            case (option.none[ebml.doc]) {
                auto pos = ebml.be_uint_from_bytes(elt.data, elt.start, 4u);
                if (eq_fn(_vec.slice[u8](elt.data, elt.start+4u, elt.end))) {
                    result = option.some[ebml.doc](ebml.doc_at(d.data, pos));
                }
            }
            case (_) {}
        }
    }
    ret result;
}

// Given a path and serialized crate metadata, returns the ID of the
// definition the path refers to.
fn resolve_path(vec[ast.ident] path, vec[u8] data) -> resolve_result {
    fn eq_item(vec[u8] data, str s) -> bool {
        ret _str.eq(_str.unsafe_from_bytes(data), s);
    }
    auto s = _str.connect(path, ".");
    auto md = ebml.new_doc(data);
    auto paths = ebml.get_doc(md, metadata.tag_paths);
    auto eqer = bind eq_item(_, s);
    alt (lookup_hash(paths, eqer, metadata.hash_path(s))) {
        case (option.some[ebml.doc](?d)) {
            auto did_doc = ebml.get_doc(d, metadata.tag_def_id);
            ret rr_ok(parse_def_id(ebml.doc_data(did_doc)));
        }
        case (option.none[ebml.doc]) {
            ret rr_not_found(s);
        }
    }
}

fn maybe_find_item(int item_id, &ebml.doc items) -> option.t[ebml.doc] {
    fn eq_item(vec[u8] bytes, int item_id) -> bool {
        ret ebml.be_uint_from_bytes(bytes, 0u, 4u) as int == item_id;
    }
    auto eqer = bind eq_item(_, item_id);
    ret lookup_hash(items, eqer, metadata.hash_def_num(item_id));
}

fn find_item(int item_id, &ebml.doc items) -> ebml.doc {
    alt (maybe_find_item(item_id, items)) {
        case (option.some[ebml.doc](?d)) {ret d;}
    }
}

// Looks up an item in the given metadata and returns an EBML doc pointing
// to the item data.
fn lookup_item(int item_id, vec[u8] data) -> ebml.doc {
    auto items = ebml.get_doc(ebml.new_doc(data), metadata.tag_items);
    ret find_item(item_id, items);
}

fn item_kind(&ebml.doc item) -> u8 {
    auto kind = ebml.get_doc(item, metadata.tag_items_data_item_kind);
    ret ebml.doc_as_uint(kind) as u8;
}

fn item_symbol(&ebml.doc item) -> str {
    auto sym = ebml.get_doc(item, metadata.tag_items_data_item_symbol);
    ret _str.unsafe_from_bytes(ebml.doc_data(sym));
}

fn variant_tag_id(&ebml.doc d) -> ast.def_id {
    auto tagdoc = ebml.get_doc(d, metadata.tag_items_data_item_tag_id);
    ret parse_def_id(ebml.doc_data(tagdoc));
}

fn item_type(&ebml.doc item, int this_cnum, @ty.type_store tystore) -> @ty.t {
    fn parse_external_def_id(int this_cnum, str s) -> ast.def_id {
        // FIXME: This is completely wrong when linking against a crate
        // that, in turn, links against another crate. We need a mapping
        // from crate ID to crate "meta" attributes as part of the crate
        // metadata.
        auto buf = _str.bytes(s);
        auto external_def_id = parse_def_id(buf);
        ret tup(this_cnum, external_def_id._1);
    }

    auto tp = ebml.get_doc(item, metadata.tag_items_data_item_type);
    auto s = _str.unsafe_from_bytes(ebml.doc_data(tp));
    ret parse_ty_str(s, bind parse_external_def_id(this_cnum, _), tystore);
}

fn item_ty_param_count(&ebml.doc item, int this_cnum) -> uint {
    let uint ty_param_count = 0u;
    auto tp = metadata.tag_items_data_item_ty_param_count;
    for each (ebml.doc p in ebml.tagged_docs(item, tp)) {
        ty_param_count = ebml.vint_at(ebml.doc_data(p), 0u)._0;
    }
    ret ty_param_count;
}

fn tag_variant_ids(&ebml.doc item, int this_cnum) -> vec[ast.def_id] {
    let vec[ast.def_id] ids = vec();
    auto v = metadata.tag_items_data_item_variant;
    for each (ebml.doc p in ebml.tagged_docs(item, v)) {
        auto ext = parse_def_id(ebml.doc_data(p));
        _vec.push[ast.def_id](ids, tup(this_cnum, ext._1));
    }
    ret ids;
}

fn get_metadata_section(str filename) -> option.t[vec[u8]] {
    auto mb = llvm.LLVMRustCreateMemoryBufferWithContentsOfFile
        (_str.buf(filename));
    if (mb as int == 0) {ret option.none[vec[u8]];}
    auto of = mk_object_file(mb);
    auto si = mk_section_iter(of.llof);
    while (llvm.LLVMIsSectionIteratorAtEnd(of.llof, si.llsi) == False) {
        auto name_buf = llvm.LLVMGetSectionName(si.llsi);
        auto name = _str.str_from_cstr(name_buf);
        if (_str.eq(name, x86.get_meta_sect_name())) {
            auto cbuf = llvm.LLVMGetSectionContents(si.llsi);
            auto csz = llvm.LLVMGetSectionSize(si.llsi);
            auto cvbuf = cbuf as _vec.vbuf;
            ret option.some[vec[u8]](_vec.vec_from_vbuf[u8](cvbuf, csz));
        }
        llvm.LLVMMoveToNextSection(si.llsi);
    }
    ret option.none[vec[u8]];
}


fn load_crate(session.session sess,
              int cnum,
              ast.ident ident,
              vec[str] library_search_paths) {
    auto filename = parser.default_native_name(sess, ident);
    for (str library_search_path in library_search_paths) {
        auto path = fs.connect(library_search_path, filename);
        alt (get_metadata_section(path)) {
            case (option.some[vec[u8]](?cvec)) {
                sess.set_external_crate(cnum, rec(name=ident, data=cvec));
                ret;
            }
            case (_) {}
        }
    }

    log_err #fmt("can't open crate '%s' (looked for '%s' in lib search path)",
                 ident, filename);
    fail;
}

fn fold_view_item_use(&env e, &span sp, ast.ident ident,
        vec[@ast.meta_item] meta_items, ast.def_id id, option.t[int] cnum_opt)
        -> @ast.view_item {
    auto cnum;
    if (!e.crate_cache.contains_key(ident)) {
        cnum = e.next_crate_num;
        load_crate(e.sess, cnum, ident, e.library_search_paths);
        e.crate_cache.insert(ident, e.next_crate_num);
        e.next_crate_num += 1;
    } else {
        cnum = e.crate_cache.get(ident);
    }

    auto viu = ast.view_item_use(ident, meta_items, id, some[int](cnum));
    ret @fold.respan[ast.view_item_](sp, viu);
}

// Reads external crates referenced by "use" directives.
fn read_crates(session.session sess,
               @ast.crate crate,
               vec[str] library_search_paths) -> @ast.crate {
    auto e = @rec(
        sess=sess,
        crate_cache=@common.new_str_hash[int](),
        library_search_paths=library_search_paths,
        mutable next_crate_num=1
    );

    auto f = fold_view_item_use;
    auto fld = @rec(fold_view_item_use=f with *fold.new_identity_fold[env]());
    ret fold.fold_crate[env](e, fld, crate);
}


fn kind_has_type_params(u8 kind_ch) -> bool {
    // FIXME: It'd be great if we had u8 char literals.
    if (kind_ch == ('c' as u8))      { ret false; }
    else if (kind_ch == ('f' as u8)) { ret true;  }
    else if (kind_ch == ('F' as u8)) { ret true;  }
    else if (kind_ch == ('y' as u8)) { ret true;  }
    else if (kind_ch == ('o' as u8)) { ret true;  }
    else if (kind_ch == ('t' as u8)) { ret true;  }
    else if (kind_ch == ('T' as u8)) { ret false;  }
    else if (kind_ch == ('m' as u8)) { ret false; }
    else if (kind_ch == ('n' as u8)) { ret false; }
    else if (kind_ch == ('v' as u8)) { ret true;  }
    else {
        log_err #fmt("kind_has_type_params(): unknown kind char: %d",
                     kind_ch as int);
        fail;
    }
}


// Crate metadata queries

fn lookup_def(session.session sess, int cnum, vec[ast.ident] path)
        -> option.t[ast.def] {
    auto data = sess.get_external_crate(cnum).data;

    auto did;
    alt (resolve_path(path, data)) {
        case (rr_ok(?di)) { did = di; }
        case (rr_not_found(?name)) {
            ret none[ast.def];
        }
    }

    auto item = lookup_item(did._1, data);
    auto kind_ch = item_kind(item);

    did = tup(cnum, did._1);

    // FIXME: It'd be great if we had u8 char literals.
    auto def;
    if (kind_ch == ('c' as u8))         { def = ast.def_const(did);      }
    else if (kind_ch == ('f' as u8))    { def = ast.def_fn(did);         }
    else if (kind_ch == ('F' as u8))    { def = ast.def_native_fn(did);  }
    else if (kind_ch == ('y' as u8))    { def = ast.def_ty(did);         }
    else if (kind_ch == ('o' as u8))    { def = ast.def_obj(did);        }
    else if (kind_ch == ('T' as u8))    { def = ast.def_native_ty(did);  }
    else if (kind_ch == ('t' as u8)) {
        // We treat references to tags as references to types.
        def = ast.def_ty(did);
    } else if (kind_ch == ('m' as u8))  { def = ast.def_mod(did);        }
    else if (kind_ch == ('n' as u8))    { def = ast.def_native_mod(did); }
    else if (kind_ch == ('v' as u8)) {
        auto tid = variant_tag_id(item);
        tid = tup(cnum, tid._1);
        def = ast.def_variant(tid, did);
    } else {
        log_err #fmt("lookup_def(): unknown kind char: %d", kind_ch as int);
        fail;
    }

    ret some[ast.def](def);
}

fn get_type(session.session sess, @ty.type_store tystore, ast.def_id def)
        -> ty.ty_param_count_and_ty {
    auto external_crate_id = def._0;
    auto data = sess.get_external_crate(external_crate_id).data;
    auto item = lookup_item(def._1, data);
    auto t = item_type(item, external_crate_id, tystore);

    auto tp_count;
    auto kind_ch = item_kind(item);
    auto has_ty_params = kind_has_type_params(kind_ch);
    if (has_ty_params) {
        tp_count = item_ty_param_count(item, external_crate_id);
    } else {
        tp_count = 0u;
    }

    ret tup(tp_count, t);
}

fn get_symbol(session.session sess, ast.def_id def) -> str {
    auto external_crate_id = def._0;
    auto data = sess.get_external_crate(external_crate_id).data;
    auto item = lookup_item(def._1, data);
    ret item_symbol(item);
}

fn get_tag_variants(session.session sess,
                    @ty.type_store tystore,
                    ast.def_id def) -> vec[trans.variant_info] {
    auto external_crate_id = def._0;
    auto data = sess.get_external_crate(external_crate_id).data;
    auto items = ebml.get_doc(ebml.new_doc(data), metadata.tag_items);
    auto item = find_item(def._1, items);

    let vec[trans.variant_info] infos = vec();
    auto variant_ids = tag_variant_ids(item, external_crate_id);
    for (ast.def_id did in variant_ids) {
        auto item = find_item(did._1, items);
        auto ctor_ty = item_type(item, external_crate_id, tystore);
        let vec[@ty.t] arg_tys = vec();
        alt (ctor_ty.struct) {
            case (ty.ty_fn(_, ?args, _)) {
                for (ty.arg a in args) {
                    arg_tys += vec(a.ty);
                }
            }
            case (_) {
                // Nullary tag variant.
            }
        }
        infos += vec(rec(args=arg_tys, ctor_ty=ctor_ty, id=did));
    }

    ret infos;
}

fn list_file_metadata(str path, io.writer out) {
    alt (get_metadata_section(path)) {
        case (option.some[vec[u8]](?bytes)) {
            list_crate_metadata(bytes, out);
        }
        case (option.none[vec[u8]]) {
            out.write_str("Could not find metadata in " + path + ".\n");
        }
    }
}

fn read_path(&ebml.doc d) -> tup(str, uint) {
    auto desc = ebml.doc_data(d);
    auto pos = ebml.be_uint_from_bytes(desc, 0u, 4u);
    auto pathbytes = _vec.slice[u8](desc, 4u, _vec.len[u8](desc));
    auto path = _str.unsafe_from_bytes(pathbytes);
    ret tup(path, pos);
}

fn list_crate_metadata(vec[u8] bytes, io.writer out) {
    auto md = ebml.new_doc(bytes);
    auto paths = ebml.get_doc(md, metadata.tag_paths);
    auto items = ebml.get_doc(md, metadata.tag_items);
    auto index = ebml.get_doc(paths, metadata.tag_index);
    auto bs = ebml.get_doc(index, metadata.tag_index_buckets);
    for each (ebml.doc bucket in
              ebml.tagged_docs(bs, metadata.tag_index_buckets_bucket)) {
        auto et = metadata.tag_index_buckets_bucket_elt;
        for each (ebml.doc elt in ebml.tagged_docs(bucket, et)) {
            auto data = read_path(elt);
            auto def = ebml.doc_at(bytes, data._1);
            auto did_doc = ebml.get_doc(def, metadata.tag_def_id);
            auto did = parse_def_id(ebml.doc_data(did_doc));
            out.write_str(#fmt("%s (%s)\n", data._0,
                               describe_def(items, did)));
        }
    }
}

fn describe_def(&ebml.doc items, ast.def_id id) -> str {
    if (id._0 != 0) {ret "external";}
    alt (maybe_find_item(id._1 as int, items)) {
        case (option.some[ebml.doc](?item)) {
            ret item_kind_to_str(item_kind(item));
        }
        case (option.none[ebml.doc]) {
            ret "??"; // Native modules don't seem to get item entries.
        }
    }
}

fn item_kind_to_str(u8 kind) -> str {
    alt (kind as char) {
        case ('c') {ret "const";}
        case ('f') {ret "fn";}
        case ('F') {ret "native fn";}
        case ('y') {ret "type";}
        case ('o') {ret "obj";}
        case ('T') {ret "native type";}
        case ('t') {ret "type";}
        case ('m') {ret "mod";}
        case ('n') {ret "native mod";}
        case ('v') {ret "tag";}
    }
}

// Local Variables:
// mode: rust
// fill-column: 78;
// indent-tabs-mode: nil
// c-basic-offset: 4
// buffer-file-coding-system: utf-8-unix
// compile-command: "make -k -C $RBUILD 2>&1 | sed -e 's/\\/x\\//x:\\//g'";
// End:
