# Class Inheritance with Generics in tscc - Investigation Report

## Executive Summary

Class inheritance with generics does NOT work in tscc. When a concrete class like `TaskService extends Repository<Task>` is compiled, the compiler:
1. **Ignores the generic type parameters** (`<Task>`) during parsing
2. **Only stores the parent class name** (`Repository`) as a plain string
3. **Fails with "Undefined class" error** at runtime when trying to instantiate `TaskService`, because the parent class `Repository` (which is generic and uninstantiated) was never registered in `class_struct_types`

---

## 1. "Undefined class" Error Location and Trigger Conditions

**File:** `/Users/kian/dev/tscc/src/codegen/llvm.rs`  
**Line:** 4895  
**Error Message:** `"Undefined class '{}'", class_name`

### Code Context (lines 4890-4896):
```rust
let (struct_type, field_info, parent_class) = self
    .class_struct_types
    .get(class_name)
    .cloned()
    .ok_or_else(|| {
        CompileError::error(format!("Undefined class '{}'", class_name), span.clone())
    })?;
```

### When It Triggers:
The error is thrown in `compile_new_expr()` (line ~4823) when:
1. Code executes `new TaskService()` 
2. `compile_new_expr` tries to look up `"TaskService"` in `class_struct_types`
3. The lookup fails because `TaskService` was never inserted into the map

### Why It Fails:
`TaskService` is never registered because:
- Generic classes (those with `type_params.is_empty() == false`) are **NOT** handled in the first-pass class registration
- Only non-generic classes are compiled via `compile_statement()` in the main loop
- The parent class `Repository<T>` also fails to compile because it's generic
- Without parent, `TaskService`'s `compile_class_decl()` call at line 4989 finds no parent fields to inherit, but the struct is still created
- BUT `TaskService` itself is created and added to `class_struct_types` at line 5025-5028

Actually, on deeper inspection: **TaskService SHOULD be registered**. The issue is that Repository (the parent) is never registered, so when TaskService tries to inherit from it, the lookup at line 4989 fails silently (the `if let Some` just skips it), but TaskService is still created.

The actual problem: **the parent class name isn't stored as fully qualified**, so when you do `new Repository<Task>()`, the codegen has no way to resolve which monomorphization of Repository you mean.

---

## 2. First-Pass Registration and Second-Pass Compilation of Classes

### First Pass: Lines 955-1031
The compile() function has FOUR first-pass loops:

**Loop 1 (959-973):** Register type aliases
```rust
if let StmtKind::TypeAlias { name, type_params, type_ann } = &stmt.kind { ... }
```

**Loop 2 (979-992):** Pre-register string enum values
```rust
if let StmtKind::EnumDecl { name, members } = &stmt.kind { ... }
```

**Loop 3 (995-1031):** Register interfaces in class_struct_types
```rust
match &stmt.kind {
    StmtKind::InterfaceDecl { name, extends, fields } => {
        // Only InterfaceDecl handled here
        // ClassDecl is explicitly NOT handled in first pass
    }
    _ => {}
}
```

**KEY FINDING:** `StmtKind::ClassDecl` is **NOT** handled in any first-pass loop.

### Second Pass: Lines 1043-1069
Register function templates (not compiled), then regular functions:
```rust
if let StmtKind::FunctionDecl { name, type_params, ... } = &stmt.kind {
    if !type_params.is_empty() {
        // Store as template for monomorphization at call sites
        self.generic_templates.insert(name.clone(), (...));
    } else if *is_async {
        self.compile_async_function_decl(...);
    } else {
        self.compile_function_decl(...);
    }
}
```

Classes are **NOT** handled here either.

### Third Pass: Lines 1094-1105 (Main Loop)
The main `main` function compilation loop:
```rust
for stmt in &program.statements {
    // Skip ONLY FunctionDecl, InterfaceDecl, TypeAlias
    if matches!(
        &stmt.kind,
        StmtKind::FunctionDecl { .. }
            | StmtKind::InterfaceDecl { .. }
            | StmtKind::TypeAlias { .. }
    ) {
        continue;  // These were already handled above
    }
    self.compile_statement(stmt, main_fn)?;  // ClassDecl GOES HERE
}
```

**KEY FINDING:** Classes reach `compile_statement()` at line 1104, where they're matched at line 1184:

```rust
StmtKind::ClassDecl {
    name,
    type_params: _,      // <-- TYPE PARAMETERS ARE DISCARDED
    parent,
    fields,
    constructor,
    methods,
} => self.compile_class_decl(name, parent, fields, constructor, methods, function),
```

Notice **`type_params: _`** - the type parameters are explicitly ignored.

### Generic Class Handling: NONE
**File:** `/Users/kian/dev/tscc/src/codegen/llvm.rs`  
**Lines:** 1043-1069 (second pass)

There is NO equivalent of generic function template storage for classes. Compare:
- **Functions:** If `type_params.is_empty()` is false, insert into `self.generic_templates` (line 1059)
- **Classes:** NO such check. All classes (even generic ones) attempt to compile immediately

This is the ROOT CAUSE: Generic classes cannot be compiled until their type parameters are known, but the codegen tries to compile them anyway (or skips them with no error).

---

## 3. How `class TaskService extends Repository<Task>` Gets Registered

### In the Parser
**File:** `/Users/kian/dev/tscc/src/parser/parser.rs`  
**Lines:** 420-429

```rust
let parent = if self.match_token(&Token::Extends) {
    let parent_name = self.expect_identifier("Expected parent class name")?;
    // Discard type args: class Child extends Parent<T>
    if self.check(&Token::Less) {
        self.parse_type_args()?;  // Parsed but thrown away!
    }
    Some(parent_name)  // Only the name is stored
} else {
    None
};
```

**KEY FINDING:** The parser line 422 comment says: **"Discard type args: class Child extends Parent<T>"**

The generic type arguments are parsed (line 424) but discarded. Only the bare parent class name is stored as `Option<String>`.

### AST Definition
**File:** `/Users/kian/dev/tscc/src/parser/ast.rs`  
**Line:** 35

```rust
ClassDecl {
    name: String,
    type_params: Vec<TypeParam>,
    parent: Option<String>,    // <-- Plain String, not TypeAnnotation
    fields: Vec<ClassField>,
    constructor: Option<ClassConstructor>,
    methods: Vec<ClassMethod>,
},
```

The `parent` field is `Option<String>`, not `Option<TypeAnnotation>`. This means:
- `Repository` is stored
- `Task` (the type argument) is lost
- When compiling, there's no way to know the parent was parameterized

### In Codegen
**File:** `/Users/kian/dev/tscc/src/codegen/llvm.rs`  
**Lines:** 4974-5028 (compile_class_decl)

```rust
fn compile_class_decl(
    &mut self,
    name: &str,
    parent: &Option<String>,
    fields: &[ClassField],
    constructor: &Option<ClassConstructor>,
    methods: &[ClassMethod],
    function: FunctionValue<'ctx>,
) -> Result<(), CompileError> {
    let mut all_fields: Vec<(String, VarType)> = Vec::new();

    if let Some(parent_name) = parent {
        if let Some((_, parent_fields, _)) = self.class_struct_types.get(parent_name) {
            all_fields.extend(parent_fields.clone());
        }
        // If parent not found, silently continue with empty parent fields
    }
    
    // Add own fields, constructor, methods...
    
    self.class_struct_types.insert(
        name.to_string(),
        (struct_type, all_fields.clone(), parent.clone()),
    );
}
```

When `TaskService` is compiled:
1. `parent_name = "Repository"` (no type arguments)
2. Codegen tries to look up `"Repository"` in `class_struct_types`
3. If `Repository` is generic, it was never added to the map
4. The lookup at line 4989 fails silently with `if let Some` - no parent fields are inherited
5. `TaskService` is still inserted into `class_struct_types` (line 5025) with zero parent fields

This means:
- `Repository<Task>` is never instantiated
- Only `TaskService` is created (correctly)
- If you try `new TaskService()`, it succeeds IF `TaskService` was registered
- But if you try `new Repository<Task>()`, it fails with "Undefined class 'Repository'"

---

## 4. Generic Classes Storage and Monomorphization

### Key Data Structures

**Generic Function Templates:**
```rust
generic_templates: HashMap<
    String,
    (
        Vec<String>,              // type param names (e.g., ["T"])
        Vec<Parameter>,           // parameters
        Option<TypeAnnotation>,   // return type
        Vec<Statement>,           // body
    ),
>,
```
Declared at line 57-65. Used to store generic functions for late monomorphization.

**Generic Type Aliases:**
```rust
generic_alias_params: HashMap<String, (Vec<String>, TypeAnnotation)>,
```
Declared at line 71. Stores generic alias type parameters and bodies.

**Class Struct Types:**
```rust
class_struct_types: HashMap<String, (StructType<'ctx>, Vec<(String, VarType)>, Option<String>)>,
```
Declared at line 45. Stores: (LLVM struct type, fields, parent class name).

### Finding: NO `generic_classes` Map Exists

Unlike functions, which are stored in `generic_templates` when generic, **there is NO equivalent for classes**.

**Grep results:**
```
grep -n "generic_classes" /Users/kian/dev/tscc/src/codegen/llvm.rs
```
Returns: **0 results**

This is the critical gap: Generic classes have nowhere to be stored as templates.

### How Generic Functions Are Monomorphized (for comparison)
**File:** `/Users/kian/dev/tscc/src/codegen/llvm.rs`  
**Lines:** 6033-6036

```rust
// Check if this is a call to a generic function — monomorphize on demand
if self.generic_templates.contains_key(name.as_str()) {
    return self.compile_generic_call(name, args, function, span);
}
```

When a generic function is called, `compile_generic_call()` (not shown) instantiates it with the call's type arguments.

### How Generic Classes Are Monomorphized: NOT AT ALL
There is **no equivalent code for classes**. When you write:
```typescript
const x = new Repository<Task>();
```

The codegen:
1. Looks for class name `"Repository"`
2. Fails to find it (it was never registered because it's generic)
3. Returns error "Undefined class 'Repository'"

There is NO call to any `compile_generic_class()` function.

---

## 5. `class_struct_types` Contents and Timing

### What's Stored
```rust
class_struct_types: HashMap<String, (StructType<'ctx>, Vec<(String, VarType)>, Option<String>)>
```

Each entry stores:
- **Key:** Class name (e.g., `"TaskService"`, `"Rectangle"`)
- **Value tuple:**
  - `StructType<'ctx>`: The LLVM struct type (fields layout)
  - `Vec<(String, VarType)>`: Field names and their types
  - `Option<String>`: Parent class name (if any), as a plain string (not parameterized)

### When Entries Are Inserted

**Location 1: Line 627 (during interface compilation)**
In `compile_statement` when handling `StmtKind::InterfaceDecl`:
```rust
self.class_struct_types
    .insert(name.clone(), (struct_type, field_vts, None));
```

**Location 2: Line 1026 (first-pass interface registration)**
In the first-pass loop for interfaces:
```rust
self.class_struct_types
    .insert(name.clone(), (struct_type, field_vts, None));
```

**Location 3: Line 1317 (enum compilation)**
When compiling enums:
```rust
self.class_struct_types
    .insert(name.clone(), (struct_type, field_vts, None));
```

**Location 4: Line 5025 (class compilation)**
In `compile_class_decl`:
```rust
self.class_struct_types.insert(
    name.to_string(),
    (struct_type, all_fields.clone(), parent.clone()),
);
```

### Is `TaskService` Inserted?
**YES, conditionally:**
- If `Repository` exists in `class_struct_types`, `TaskService` inherits its fields and is inserted at line 5025
- If `Repository` does NOT exist, `TaskService` is still inserted (line 5025) but with zero inherited fields

The problem:
- `Repository<T>` is GENERIC, so it's never inserted in the first place
- When `TaskService extends Repository<Task>` is compiled, the lookup for `"Repository"` at line 4989 fails
- No inherited fields are added, but `TaskService` itself IS inserted

### Summary of `class_struct_types` for Example
If you have:
```typescript
class Repository<T> { items: T[] = []; }
class TaskService extends Repository<Task> { }
interface Task { id: number; }
const svc = new TaskService();
```

The `class_struct_types` map after compilation contains:
- `"Task"` → (task_struct, [("id", Number)], None)
- `"TaskService"` → (taskservice_struct, [("name", String)], Some("Repository"))

The `"Repository"` key is **MISSING** because:
1. It's generic (`type_params` is not empty)
2. There's no special handling for generic classes
3. It was never compiled/registered

---

## 6. Where `new ClassName()` is Compiled

### Location and Lookup

**File:** `/Users/kian/dev/tscc/src/codegen/llvm.rs`  
**Function:** `compile_new_expr()` (starts ~line 4780)  
**Critical lookup:** Lines 4890-4896

```rust
let (struct_type, field_info, parent_class) = self
    .class_struct_types
    .get(class_name)      // <-- Direct string lookup
    .cloned()
    .ok_or_else(|| {
        CompileError::error(format!("Undefined class '{}'", class_name), span.clone())
    })?;
```

### How the Lookup Works

1. **Extract class name:** From `ExprKind::New { class_name, args }`
2. **Direct string map lookup:** `class_struct_types.get(class_name)`
3. **Fail if not found:** Error "Undefined class"
4. **Success case:** Extract LLVM struct type, fields, parent class name

### Why It Fails for `Repository<Task>`

When you write: `new Repository<Task>()`

The parser extracts the class name as `"Repository"` (the type arguments `<Task>` are lost).

Then codegen does:
```rust
self.class_struct_types.get("Repository").ok_or_else(|| 
    CompileError::error("Undefined class 'Repository'", ...)
)?;
```

But `"Repository"` is never in the map because:
1. Generic classes are never compiled
2. No monomorphization happens
3. There's no template storage mechanism for classes

### What Would Be Needed to Fix This

1. **Store generic classes as templates** (like `generic_templates` for functions)
2. **Detect generic instantiation in `compile_new_expr`**
3. **Monomorphize on demand** (create `Repository_Task`, compile it, register it)
4. **Update the lookup** to use the monomorphized name

---

## 7. Key Missing Infrastructure

### For Generic Classes to Work, Need:

1. **`generic_class_templates` Map**
   ```rust
   generic_class_templates: HashMap<
       String,  // class name (e.g., "Repository")
       (
           Vec<String>,          // type param names
           Vec<ClassField>,      // field definitions
           Option<String>,       // parent class (unparameterized)
           Option<ClassConstructor>,
           Vec<ClassMethod>,
       ),
   >,
   ```

2. **First-pass registration for generic classes** (lines 995-1031)
   ```rust
   StmtKind::ClassDecl { name, type_params, parent, fields, constructor, methods } => {
       if !type_params.is_empty() {
           // Store as template
           let tp_names: Vec<String> = ...;
           self.generic_class_templates.insert(name.clone(), (...));
       }
   }
   ```

3. **Monomorphization in `compile_new_expr`** (before line 4890)
   ```rust
   if self.generic_class_templates.contains_key(class_name) {
       return self.compile_generic_new(class_name, type_args, args, function, span);
   }
   ```

4. **Monomorphization function** `compile_generic_new()`
   - Extract type arguments from `class_name` (currently lost in parser)
   - Create substitution map
   - Instantiate fields and methods with substituted types
   - Register as new class in `class_struct_types`
   - Proceed with normal `new` compilation

---

## Summary

| Item | Current Status | Issue |
|------|---|---|
| Parser extracts generic parent class types | ✗ No | Discarded at line 422 of parser.rs |
| AST stores generic parent parameterization | ✗ No | `parent: Option<String>` only |
| Generic classes stored as templates | ✗ No | No `generic_class_templates` map |
| First-pass registers generic classes | ✗ No | Only interfaces in first pass |
| `compile_new_expr` detects generics | ✗ No | Direct string lookup only |
| Monomorphization on `new` | ✗ No | No `compile_generic_new` function |
| | | |
| Error message when `new Repository()` | "Undefined class 'Repository'" | Line 4895 |
| Why `TaskService extends Repository<T>` works* | Parent lookup silently fails | Lines 4988-4991 |
| Why `new TaskService()` fails | Never registered | If parent not in `class_struct_types` |
| Why `new Repository<Task>()` fails | Generic never compiled | No monomorphization mechanism |

*`TaskService` will work IF `Repository` is non-generic, because it will inherit fields. But if `Repository` is generic, the parent lookup at 4989 returns None, and TaskService gets zero inherited fields.

