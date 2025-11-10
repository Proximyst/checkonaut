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
	assert(#result == 1, "Expected exactly one reason for invalidity")
	assert(result[1] == "object is invalid", "Unexpected reason for invalidity: " .. tostring(result[1]))
end
