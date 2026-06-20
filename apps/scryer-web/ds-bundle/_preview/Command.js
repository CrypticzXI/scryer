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

  // .design-sync/previews/Command.tsx
  var Command_exports = {};
  __export(Command_exports, {
    Palette: () => Palette
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

  // .design-sync/previews/Command.tsx
  var import_jsx_runtime = __toESM(require_react_shim(), 1);
  function Palette() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "p-6", children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Command, { className: "max-w-md rounded-lg border border-border shadow-md", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandInput, { placeholder: "Search movies, series, settings…" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.CommandList, { children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandEmpty, { children: "No results found." }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.CommandGroup, { heading: "Library", children: [
          /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.CommandItem, { children: [
            "Add movie",
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandShortcut, { children: "⌘A" })
          ] }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandItem, { children: "Search wanted" }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandItem, { children: "Refresh all" })
        ] }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandSeparator, {}),
        /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.CommandGroup, { heading: "Go to", children: [
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandItem, { children: "Activity" }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandItem, { children: "Calendar" }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.CommandItem, { children: [
            "Settings",
            /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CommandShortcut, { children: "⌘," })
          ] })
        ] })
      ] })
    ] }) });
  }
  return __toCommonJS(Command_exports);
})();
