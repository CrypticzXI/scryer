"use strict";
var __dsPreview = (() => {
  var __create = Object.create;
  var __defProp = Object.defineProperty;
  var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
  var __getOwnPropNames = Object.getOwnPropertyNames;
  var __getProtoOf = Object.getPrototypeOf;
  var __hasOwnProp = Object.prototype.hasOwnProperty;
  var __esm = (fn, res, err) => function __init() {
    if (err) throw err[0];
    try {
      return fn && (res = (0, fn[__getOwnPropNames(fn)[0]])(fn = 0)), res;
    } catch (e) {
      throw err = [e], e;
    }
  };
  var __commonJS = (cb, mod) => function __require() {
    try {
      return mod || (0, cb[__getOwnPropNames(cb)[0]])((mod = { exports: {} }).exports, mod), mod.exports;
    } catch (e) {
      throw mod = 0, e;
    }
  };
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, { get: all[name], enumerable: true });
  };
  var __copyProps = (to, from, except, desc) => {
    if (from && typeof from === "object" || typeof from === "function") {
      for (let key of __getOwnPropNames(from))
        if (!__hasOwnProp.call(to, key) && key !== except)
          __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
    }
    return to;
  };
  var __reExport = (target, mod, secondTarget) => (__copyProps(target, mod, "default"), secondTarget && __copyProps(secondTarget, mod, "default"));
  var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
    // If the importer is in node compatibility mode or this is not an ESM
    // file that has been converted to a CommonJS file using a Babel-
    // compatible transform (i.e. "__esModule" has not been set), then set
    // "default" to the CommonJS "module.exports" for node compatibility.
    isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
    mod
  ));
  var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

  // <define:import.meta.env>
  var init_define_import_meta_env = __esm({
    "<define:import.meta.env>"() {
    }
  });

  // ds-raw:__ds_raw__
  var require_ds_raw = __commonJS({
    "ds-raw:__ds_raw__"(exports, module) {
      init_define_import_meta_env();
      module.exports = window.ScryerWeb;
    }
  });

  // shim:react-shim
  var require_react_shim = __commonJS({
    "shim:react-shim"(exports, module) {
      init_define_import_meta_env();
      var R = window.React;
      function jsx2(t, p, k) {
        return R.createElement(t, k === void 0 ? p : Object.assign({ key: k }, p));
      }
      module.exports = R;
      module.exports.jsx = jsx2;
      module.exports.jsxs = jsx2;
      module.exports.jsxDEV = jsx2;
      module.exports.Fragment = R.Fragment;
    }
  });

  // .design-sync/previews/Table.tsx
  var Table_exports = {};
  __export(Table_exports, {
    Movies: () => Movies
  });
  init_define_import_meta_env();

  // ds-shim:ds
  var ds_exports = {};
  __export(ds_exports, {
    default: () => ds_default
  });
  init_define_import_meta_env();
  __reExport(ds_exports, __toESM(require_ds_raw()));
  var g = window.ScryerWeb;
  var ds_default = "default" in g ? g.default : g;

  // .design-sync/previews/Table.tsx
  var import_jsx_runtime = __toESM(require_react_shim(), 1);
  var rows = [
    { title: "Dune: Part Two", year: 2024, quality: "2160p", status: "Downloaded" },
    { title: "Oppenheimer", year: 2023, quality: "1080p", status: "Downloaded" },
    { title: "The Batman", year: 2022, quality: "1080p", status: "Missing" },
    { title: "Blade Runner 2049", year: 2017, quality: "2160p", status: "Monitored" }
  ];
  function Movies() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "p-6", children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Table, { children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableHeader, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.TableRow, { children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableHead, { children: "Title" }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableHead, { children: "Year" }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableHead, { children: "Quality" }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableHead, { children: "Status" })
      ] }) }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableBody, { children: rows.map((r) => /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.TableRow, { children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableCell, { className: "font-medium text-foreground", children: r.title }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableCell, { className: "text-muted-foreground", children: r.year }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableCell, { className: "text-muted-foreground", children: r.quality }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.TableCell, { className: "text-muted-foreground", children: r.status })
      ] }, r.title)) })
    ] }) });
  }
  return __toCommonJS(Table_exports);
})();
