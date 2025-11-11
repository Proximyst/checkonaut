local checkonaut = require("@checkonaut")

function Check(obj)
	local keysToExist = checkonaut.ReadJSON("external.json")
	for _, key in ipairs(keysToExist) do
		if obj[key] == nil then
			return "Missing key: " .. key
		end
	end
	return nil
end
