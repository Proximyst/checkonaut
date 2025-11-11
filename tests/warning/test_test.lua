require("./test")

function TestReturnsInvalid()
	local obj = {
		apiVersion = "v1",
		kind = "Junk",
		metadata = {
			name = "invalid-object",
		},
	}
	local result = Check(obj)
	assert(type(result) == "table", "Expected result to be a table")
	assert(result["severity"] == "warning", "Unexpected severity: " .. tostring(result["severity"]))
	assert(result["message"] == "object is invalid", "Unexpected message: " .. tostring(result["message"]))
end
