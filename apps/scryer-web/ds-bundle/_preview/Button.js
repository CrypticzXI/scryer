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

  // .design-sync/previews/Button.tsx
  var Button_exports = {};
  __export(Button_exports, {
    IconButtons: () => IconButtons,
    Sizes: () => Sizes,
    States: () => States,
    Variants: () => Variants,
    WithIcon: () => WithIcon
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

  // .design-sync/previews/Button.tsx
  var import_jsx_runtime = __toESM(require_react_shim(), 1);
  var Plus = () => /* @__PURE__ */ (0, import_jsx_runtime.jsx)("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true, children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M5 12h14M12 5v14" }) });
  var Search = () => /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true, children: [
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("circle", { cx: "11", cy: "11", r: "8" }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "m21 21-4.3-4.3" })
  ] });
  var Refresh = () => /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("svg", { viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: "2", strokeLinecap: "round", strokeLinejoin: "round", "aria-hidden": true, children: [
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M3 12a9 9 0 0 1 15-6.7L21 8" }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M21 3v5h-5" }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M21 12a9 9 0 0 1-15 6.7L3 16" }),
    /* @__PURE__ */ (0, import_jsx_runtime.jsx)("path", { d: "M3 21v-5h5" })
  ] });
  function Variants() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { className: "flex flex-wrap items-center gap-3 p-6", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { children: "Add Movie" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { variant: "secondary", children: "Refresh" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { variant: "outline", children: "Search" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { variant: "ghost", children: "Cancel" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { variant: "destructive", children: "Delete" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { variant: "link", children: "View details" })
    ] });
  }
  function Sizes() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { className: "flex flex-wrap items-center gap-3 p-6", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "sm", children: "Small" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "default", children: "Default" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "lg", children: "Large" })
    ] });
  }
  function WithIcon() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { className: "flex flex-wrap items-center gap-3 p-6", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Button, { children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Plus, {}),
        "Add Movie"
      ] }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Button, { variant: "outline", children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Search, {}),
        "Search Indexers"
      ] }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Button, { variant: "secondary", children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Refresh, {}),
        "Refresh Library"
      ] })
    ] });
  }
  function IconButtons() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { className: "flex flex-wrap items-center gap-3 p-6", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "icon", "aria-label": "Add", children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Plus, {}) }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "icon", variant: "outline", "aria-label": "Search", children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Search, {}) }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "icon", variant: "ghost", "aria-label": "Refresh", children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(Refresh, {}) })
    ] });
  }
  function States() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { className: "flex flex-wrap items-center gap-3 p-6", children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { children: "Enabled" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { disabled: true, children: "Disabled" }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { variant: "outline", disabled: true, children: "Unavailable" })
    ] });
  }
  return __toCommonJS(Button_exports);
})();
