# checkonaut

Checkonaut lets you easily write checks against structured data, such as
Kubernetes resources or configuration files.

## Usage

To use checkonaut, first install it. It is a Rust application, so `cargo
install` is a good way to go about this.

You should decide where to write your scripts and where your data is; these are
often semantically linked, so they should live near each other. For example, if
you generate Kubernetes YAML resources into a central directory for use with
Argo, you likely want the checks right next to these files in another directory.

Your checks are simple Lua files. They have one function, `Check(object)`, where
we will call `Check` for every single object in every single file in your
dataset. If no `Check` exists, it is assumed to be a library file, and is not
used for checking; if the file ends with `_test.lua`, it is assumed to be
testing the check, hence it does not get run for normal operations.

As an example, you can check that all Kubernetes `Namespace`s have a
`metadata.name` like this:

```lua
function Check(obj)
  -- The Check function is the entrypoint for your check.
  -- It must return an array of strings/nils, a string, or a nil.
  -- Any non-nil value is regarded as an error to print about the object.
  -- This means one check can do many different assertions and return them all.

  if not obj or obj.kind ~= "Namespace" then
    return nil
  end

  if not obj.metadata or not obj.metadata.name then
    return "Namespace is missing metadata.name"
  end

  return nil
end
```

And likewise, you can test that this will work with unit tests. This is done via
`_test.lua` files, where all functions starting with `Test` are run. The `Check`
function is implicitly required here, so you don't need to `require` it. For
example:

```lua
function TestNonKubernetesObject()
  local res = Check({ key = "Value" })
  assert(res == nil)
end

function TestMissingName()
  local res = Check({ kind = "Namespace", metadata = {} })
  assert(res ~= nil)
  assert(res[1] == "Namespace is missing metadata.name")
end

function TestValidNamespace()
  local res = Check({ kind = "Namespace", metadata = { name = "valid-name" } })
  assert(res == nil)
end
```

What if the check shouldn't be blocking yet, though, as it's still in partial
implementation? We lied a bit above: you can also return tables, or even arrays
of tables:

```lua
function Check(obj)
  if not obj or obj.kind ~= "Namespace" then
    return nil
  end

  local issues = {}

  if not obj.metadata or not obj.metadata.name then
    table.insert(issues, {
      message = "Namespace is missing metadata.name",
      severity = "warning",
    })
  end

  return issues
end
```

Check is actually also called with _two_ parameters, if it asks for it: one is
the raw object, and the other is a context object, containing metadata about the
invocation:

```lua
Check(
  {}, -- The decoded table of the file.
  {
    file = {
      path = "/absolute/path/to/file.yaml", -- This is from the root of the system.
      -- More details may be added in the future.
    },
    check = {
      path = "/absolute/path/to/check.lua", -- The check file being executed.
      -- More details may be added in the future.
    },
  }, -- Context; may be nil if you don't pass it in inside your Test functions!
)
```

The context can be useful if you require a way to add exceptions while a check is
in the works of being implemented.
