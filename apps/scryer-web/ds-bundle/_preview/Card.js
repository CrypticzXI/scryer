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

  // .design-sync/previews/Card.tsx
  var Card_exports = {};
  __export(Card_exports, {
    Basic: () => Basic,
    Stats: () => Stats,
    WithActions: () => WithActions
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

  // .design-sync/previews/Card.tsx
  var import_jsx_runtime = __toESM(require_react_shim(), 1);
  function Basic() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "p-6 max-w-md", children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Card, { children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CardHeader, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CardTitle, { children: "Quality Profile" }) }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CardContent, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", { className: "text-sm text-muted-foreground", children: "Releases are upgraded until they reach your cutoff. Higher-ranked qualities are preferred when multiple releases are available." }) })
    ] }) });
  }
  function Stats() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "grid grid-cols-3 gap-4 p-6", children: [
      { label: "Movies", value: "1,284" },
      { label: "Monitored", value: "1,107" },
      { label: "Missing", value: "63" }
    ].map((s) => /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Card, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.CardContent, { children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "text-2xl font-semibold text-foreground", children: s.value }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "text-xs text-muted-foreground mt-1", children: s.label })
    ] }) }, s.label)) });
  }
  function WithActions() {
    return /* @__PURE__ */ (0, import_jsx_runtime.jsx)("div", { className: "p-6 max-w-md", children: /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.Card, { children: [
      /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CardHeader, { children: /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.CardTitle, { children: "Download Client" }) }),
      /* @__PURE__ */ (0, import_jsx_runtime.jsxs)(ds_exports.CardContent, { children: [
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)("p", { className: "text-sm text-muted-foreground", children: "qBittorrent · http://localhost:8080 · connected" }),
        /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Separator, {}),
        /* @__PURE__ */ (0, import_jsx_runtime.jsxs)("div", { className: "flex items-center gap-2", children: [
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "sm", children: "Test" }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "sm", variant: "outline", children: "Edit" }),
          /* @__PURE__ */ (0, import_jsx_runtime.jsx)(ds_exports.Button, { size: "sm", variant: "ghost", children: "Remove" })
        ] })
      ] })
    ] }) });
  }
  return __toCommonJS(Card_exports);
})();
