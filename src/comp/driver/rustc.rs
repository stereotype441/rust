// -*- rust -*-

import front.creader;
import front.parser;
import front.token;
import front.eval;
import middle.trans;
import middle.resolve;
import middle.capture;
import middle.ty;
import middle.typeck;
import middle.typestate_check;
import util.common;

import std.map.mk_hashmap;
import std.option;
import std.option.some;
import std.option.none;
import std._str;
import std._vec;
import std.io;

fn default_environment(session.session sess,
                       str argv0,
                       str input) -> eval.env {

    auto libc = "libc.so";
    alt (sess.get_targ_cfg().os) {
        case (session.os_win32) { libc = "msvcrt.dll"; }
        case (session.os_macos) { libc = "libc.dylib"; }
        case (session.os_linux) { libc = "libc.so.6"; }
    }

    ret
        vec(
            // Target bindings.
            tup("target_os", eval.val_str(std.os.target_os())),
            tup("target_arch", eval.val_str("x86")),
            tup("target_libc", eval.val_str(libc)),

            // Build bindings.
            tup("build_compiler", eval.val_str(argv0)),
            tup("build_input", eval.val_str(input))
            );
}

fn parse_input(session.session sess,
                      parser.parser p,
                      str input) -> @front.ast.crate {
    if (_str.ends_with(input, ".rc")) {
        ret parser.parse_crate_from_crate_file(p);
    } else if (_str.ends_with(input, ".rs")) {
        ret parser.parse_crate_from_source_file(p);
    }
    sess.err("unknown input file type: " + input);
    fail;
}

fn compile_input(session.session sess,
                        eval.env env,
                        str input, str output,
                        bool shared,
                        bool optimize,
                        bool verify,
                        trans.output_type ot,
                        vec[str] library_search_paths) {
    auto def = tup(0, 0);
    auto p = parser.new_parser(sess, env, def, input, 0u);
    auto crate = parse_input(sess, p, input);
    if (ot == trans.output_type_none) {ret;}

    crate = creader.read_crates(sess, crate, library_search_paths);
    crate = resolve.resolve_crate(sess, crate);
    capture.check_for_captures(sess, crate);

    auto tystore = ty.mk_type_store();
    auto typeck_result = typeck.check_crate(sess, tystore, crate);
    crate = typeck_result._0;
    auto type_cache = typeck_result._1;
    // FIXME: uncomment once typestate_check works
    // crate = typestate_check.check_crate(crate);
    trans.trans_crate(sess, crate, tystore, type_cache, output, shared,
                      optimize, verify, ot);
}

fn pretty_print_input(session.session sess,
                             eval.env env,
                             str input) {
    auto def = tup(0, 0);
    auto p = front.parser.new_parser(sess, env, def, input, 0u);
    auto crate = front.parser.parse_crate_from_source_file(p);
    pretty.pprust.print_file(crate.node.module, input, std.io.stdout());
}

fn warn_wrong_compiler() {
    io.stdout().write_str("This is the rust 'self-hosted' compiler.
The one written in rust.
It is currently incomplete.
You may want rustboot instead, the compiler next door.\n");
}

fn usage(session.session sess, str argv0) {
    io.stdout().write_str(#fmt("usage: %s [options] <input>\n", argv0) + "
options:

    -o <filename>      write output to <filename>
    -nowarn            suppress wrong-compiler warning
    -glue              generate glue.bc file
    -shared            compile a shared-library crate
    -pp                pretty-print the input instead of compiling
    -ls                list the symbols defined by a crate file
    -L <path>          add a directory to the library search path
    -noverify          suppress LLVM verification step (slight speedup)
    -h                 display this message\n\n");
}

fn get_os() -> session.os {
    auto s = std.os.target_os();
    if (_str.eq(s, "win32")) { ret session.os_win32; }
    if (_str.eq(s, "macos")) { ret session.os_macos; }
    if (_str.eq(s, "linux")) { ret session.os_linux; }
}

fn main(vec[str] args) {

    // FIXME: don't hard-wire this.
    auto target_cfg = rec(os = get_os(),
                          arch = session.arch_x86,
                          int_type = common.ty_i32,
                          uint_type = common.ty_u32,
                          float_type = common.ty_f64 );

    auto crate_cache = common.new_int_hash[session.crate_metadata]();
    auto target_crate_num = 0;
    let vec[@front.ast.meta_item] md = vec();
    auto sess = session.session(target_crate_num, target_cfg, crate_cache,
                                md, front.codemap.new_codemap());

    let option.t[str] input_file = none[str];
    let option.t[str] output_file = none[str];
    let vec[str] library_search_paths = vec();
    let bool do_warn = true;
    let bool shared = false;
    let bool pretty = false;
    let bool ls = false;
    auto ot = trans.output_type_bitcode;
    let bool glue = false;
    let bool verify = true;

    // FIXME: Maybe we should support -O0, -O1, -Os, etc
    let bool optimize = false;

    auto i = 1u;
    auto len = _vec.len[str](args);

    // FIXME: a getopt module would be nice.
    while (i < len) {
        auto arg = args.(i);
        if (_str.byte_len(arg) > 0u && arg.(0) == '-' as u8) {
            if (_str.eq(arg, "-nowarn")) {
                do_warn = false;
            } else if (_str.eq(arg, "-O")) {
                optimize = true;
            } else if (_str.eq(arg, "-glue")) {
                glue = true;
            } else if (_str.eq(arg, "-shared")) {
                shared = true;
            } else if (_str.eq(arg, "-pp")) {
                pretty = true;
            } else if (_str.eq(arg, "-ls")) {
                ls = true;
            } else if (_str.eq(arg, "-parse-only")) {
                ot = trans.output_type_none;
            } else if (_str.eq(arg, "-S")) {
                ot = trans.output_type_assembly;
            } else if (_str.eq(arg, "-c")) {
                ot = trans.output_type_object;
            } else if (_str.eq(arg, "-o")) {
                if (i+1u < len) {
                    output_file = some(args.(i+1u));
                    i += 1u;
                } else {
                    usage(sess, args.(0));
                    sess.err("-o requires an argument");
                }
            } else if (_str.eq(arg, "-L")) {
                if (i+1u < len) {
                    library_search_paths += vec(args.(i+1u));
                    i += 1u;
                } else {
                    usage(sess, args.(0));
                    sess.err("-L requires an argument");
                }
            } else if (_str.eq(arg, "-noverify")) {
                verify = false;
            } else if (_str.eq(arg, "-h")) {
                usage(sess, args.(0));
            } else {
                usage(sess, args.(0));
                sess.err("unrecognized option: " + arg);
            }
        } else {
            alt (input_file) {
                case (some[str](_)) {
                    usage(sess, args.(0));
                    sess.err("multiple inputs provided");
                }
                case (none[str]) {
                    input_file = some[str](arg);
                }
            }
        }
        i += 1u;
    }

    if (do_warn) {
        warn_wrong_compiler();
    }

    if (glue) {
        alt (output_file) {
            case (none[str]) {
                middle.trans.make_common_glue("glue.bc", optimize, verify,
                                              ot);
            }
            case (some[str](?s)) {
                middle.trans.make_common_glue(s, optimize, verify, ot);
            }
        }
        ret;
    }

    alt (input_file) {
        case (none[str]) {
            usage(sess, args.(0));
            sess.err("no input filename");
        }
        case (some[str](?ifile)) {

            auto env = default_environment(sess, args.(0), ifile);
            if (pretty) {
                pretty_print_input(sess, env, ifile);
            } else if (ls) {
                front.creader.list_file_metadata(ifile, std.io.stdout());
            } else {
                alt (output_file) {
                    case (none[str]) {
                        let vec[str] parts = _str.split(ifile, '.' as u8);
                        _vec.pop[str](parts);
                        parts += vec(".bc");
                        auto ofile = _str.concat(parts);
                        compile_input(sess, env, ifile, ofile, shared,
                                      optimize, verify, ot,
                                      library_search_paths);
                    }
                    case (some[str](?ofile)) {
                        compile_input(sess, env, ifile, ofile, shared,
                                      optimize, verify, ot,
                                      library_search_paths);
                    }
                }
            }
        }
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
