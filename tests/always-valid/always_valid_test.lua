function TestReturnsNil()
	require("returns_nil")
	assert(Check({}) == nil, "Check did not return nil")
end

function TestReturnsEmpty()
	require("returns_empty")
	local result = Check({})
	assert(type(result) == "table", "Check did not return a table")
	assert(#result == 0, "Check did not return an empty table")
end
