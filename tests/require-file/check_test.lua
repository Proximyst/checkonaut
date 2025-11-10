require("check")

function TestCheck()
	local result = Check({})
	assert(type(result) == "table", "Check should return a table")
	assert(#result == 0, "Check should return an empty table")
end
